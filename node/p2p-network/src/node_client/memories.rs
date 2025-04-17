use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tracing::{info, debug, error, warn};

use crate::types::{
    Reverie,
    ReverieId,
    ReverieType,
    SignatureType,
    VerifyingKey,
};
use runtime::llm::{
    ToolUsageMetrics,
    call_anthropic_record_metrics,
    call_deepseek_record_metrics
};

use super::NodeClient;


impl<'a> NodeClient<'a> {

    /// Client sends some ReverieType over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn spawn_memory_reverie(
        &mut self,
        memory_secrets: serde_json::Value,
        threshold: usize,
        total_frags: usize,
        verifying_public_key: VerifyingKey,
    ) -> Result<Reverie> {

        if threshold > total_frags {
            return Err(anyhow!("Threshold must be less than or equal to total fragments"));
        }

        // get list of target vessel and kfrag provider nodes
        let (
            target_vessel,
            target_kfrag_providers
        ) = self.get_prospect_vessels(false).await?;

        // Create a "Reverie"––an encrypted memory that alters how a Host behaves
        let reverie = self.create_reverie(
            memory_secrets,
            ReverieType::Memory,
            threshold,
            total_frags,
            target_vessel.umbral_public_key,
            verifying_public_key
        )?;

        self.broadcast_reverie_keyfrags(
            &reverie,
            target_vessel.peer_id,
            target_kfrag_providers
        ).await?;

        info!("RequestResponse broadcast of kfrags complete.");
        Ok(reverie)
    }

    // This needs to happen within the TEE as there is a decryption step, and the original
    // plaintext is revealed within the TEE, before executing with it as context
    async fn reconstruct_memory_cfrags<T: Serialize + DeserializeOwned>(
        &mut self,
        reverie_id: ReverieId,
        reverie_type: ReverieType,
        prev_failed_vessel_peer_id: libp2p::PeerId,
        signature: SignatureType
    ) -> Result<T> {

        let reverie_msg = self.get_reverie(reverie_id.clone(), reverie_type).await?;
        let capsule = reverie_msg.reverie.encode_capsule()?;
        let keyfrag_providers = reverie_msg.keyfrag_providers.clone();

        let cfrags_raw = self.request_cfrags(
            reverie_id.clone(),
            keyfrag_providers,
            signature
        ).await;

        let (
            verified_cfrags,
            source_pubkey,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw, capsule.clone())?;

        let next_agent_secrets = self.decrypt_cfrags(
            capsule,
            reverie_msg.reverie.umbral_ciphertext,
            source_pubkey,
            verified_cfrags
        );

        next_agent_secrets
    }

    pub async fn execute_with_memory_reverie(
        &mut self,
        reverie_id: ReverieId,
        reverie_type: ReverieType,
        signature: SignatureType
    ) -> Result<()> {

        let memory_secrets_json: serde_json::Value = self.reconstruct_memory_cfrags(
            reverie_id.clone(),
            reverie_type,
            self.node_id.peer_id,
            signature
        ).await?;

        let use_weather_mcp = memory_secrets_json.get("use_weather_mcp")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Initialize metrics tracker
        let mut metrics = ToolUsageMetrics::default();

        // Generate a weather-related prompt if weather MCP is enabled
        let base_prompt = "Recite a poem based on one of your memories";
        let weather_prompt = if use_weather_mcp {
            "
            Please follow these instructions:
            1. Select ONE random memory from my memories. Try not to use the same memory twice.
            2. Check the current weather forecast for the location associated with that memory (using the latitude and longitude coordinates).
            3. If the location is in one of the states listed in 'states_to_monitor', also check for any weather alerts in that state.
            4. Create a poem that blends my original memory with the current weather conditions at that location.
            5. Begin your response by stating which memory you chose and the weather you found.
            "
        } else {
            base_prompt
        };

        // Then execute LLM with Reverie as context:
        // 1. Test LLM API key from decrypted Reverie works
        if let Some(_anthropic_api_key) = memory_secrets_json["anthropic_api_key"].as_str() {
            info!("Decrypted Anthropic API key, querying Claude...");
            metrics.record_attempt();

            match call_anthropic_record_metrics(
                _anthropic_api_key,
                weather_prompt, // Use the weather-related prompt
                &memory_secrets_json["memories"].to_string(),
                &mut metrics
            ).await {
                Ok(result) => {
                    info!("\n{} {}\n", "Claude:".bright_black(), result.text.yellow());
                },
                Err(e) => {
                    warn!("Failed to call Anthropic API: {}", e);
                }
            }
        }

        // 2. Test DeepSeek API if key is available
        if let Some(deepseek_api_key) = memory_secrets_json["deepseek_api_key"].as_str() {
            info!("Decrypted DeepSeek API key, querying DeepSeek...");
            metrics.record_attempt();

            match call_deepseek_record_metrics(
                deepseek_api_key,
                weather_prompt, // Use the weather-related prompt
                &memory_secrets_json["memories"].to_string(),
                &mut metrics
            ).await {
                Ok(result) => {
                    info!("\n{} {}\n", "DeepSeek:".bright_black(), result.text.green());
                },
                Err(e) => {
                    warn!("Failed to call DeepSeek API: {}", e);
                }
            }
        }

        let report = metrics.generate_report();
        info!("{}", report);

        Ok(())
    }

}
