use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tracing::{info, debug, error, warn};

use crate::types::{
    Reverie,
    ReverieId,
    ReverieType,
    AccessCondition,
    AccessKey,
};
use runtime::llm::{
    MCPToolUsageMetrics,
    call_anthropic,
    call_deepseek
};
use super::NodeClient;



impl NodeClient {

    /// Client sends some ReverieType over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn spawn_memory_reverie(
        &mut self,
        memory_secrets: serde_json::Value,
        threshold: usize,
        total_frags: usize,
        user_public_key: AccessCondition, // access condition for using the memory
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
            target_vessel.umbral_verifying_public_key,
            user_public_key // access_pubkey to be checked against to request cfrags
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
    async fn reconstruct_memory_reverie<T: Serialize + DeserializeOwned>(
        &mut self,
        reverie_id: &ReverieId,
        reverie_type: ReverieType,
        prev_failed_vessel_peer_id: libp2p::PeerId,
        access_key: AccessKey
    ) -> Result<(T, AccessKey)> {

        let reverie_msg = self.get_reverie(reverie_id, reverie_type).await?;
        let capsule = reverie_msg.reverie.encode_capsule()?;
        let keyfrag_providers = reverie_msg.keyfrag_providers.clone();

        let cfrags_raw = self.request_cfrags(
            reverie_id,
            keyfrag_providers,
            access_key.clone()
        ).await;

        let (
            verified_cfrags,
            source_pubkey,
            target_verifying_pubkey, // target public intended to decrypt the ciphertext
            access_condition,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw, capsule.clone())?;

        let next_agent_secrets = self.decrypt_cfrags(
            capsule,
            reverie_msg.reverie.umbral_ciphertext,
            source_pubkey,
            verified_cfrags
        )?;

        Ok((next_agent_secrets, access_key))
    }

    pub async fn delegate_api_key(
        &mut self,
        reverie_id: ReverieId,
        reverie_type: ReverieType,
        access_key: AccessKey,
        // Dev's signature required to access the delegated API key
    ) -> Result<()> {

        let (
            api_keys_json,
            spenders_address // user using the memory
        ) = self.reconstruct_memory_reverie::<serde_json::Value>(
            &reverie_id,
            reverie_type,
            self.node_id.peer_id,
            access_key
        ).await?;

        let anthropic_api_key = match api_keys_json["anthropic_api_key"].as_str() {
            Some(key) => key,
            None => {
                return Err(anyhow!("No Anthropic API key found in decrypted Reverie. Delegation failed."));
            }
        };

        println!("Decrypted Anthropic API key, delegating...");
        let opt_ca_cert = self.llm_proxy_ca_cert.read().await.clone();
        if let Some(ca_cert) = opt_ca_cert {
            self.add_proxy_api_key(
                reverie_id.clone(),
                "ANTHROPIC_API_KEY".to_string(),
                anthropic_api_key.to_string(),
                spenders_address.to_string(),
                spenders_address.get_type(),
            ).await?;
        } else {
            return Err(anyhow!("No CA certificate found. Delegation failed."));
        }

        Ok(())
    }

    pub async fn execute_with_memory_reverie(
        &mut self,
        reverie_id: ReverieId,
        reverie_type: ReverieType,
        access_key: AccessKey, // Auth required to access the memory
        anthropic_query: AnthropicQuery
    ) -> Result<ExecuteWithMemoryReverieResult> {

        let (
            memory_secrets_json,
            spenders_address // user using the memory
        ) = self.reconstruct_memory_reverie::<serde_json::Value>(
            &reverie_id,
            reverie_type,
            self.node_id.peer_id,
            access_key
        ).await?;

        let mut metrics = MCPToolUsageMetrics::default();

        // Then execute LLM with Reverie as context:
        // 1. Test LLM API key from decrypted Reverie works
        println!("Decrypted secret memory, querying Claude with private contexts...");
        metrics.record_attempt();
        let node_keypair = &self.node_id.id_keys.clone();
        let secret_context = memory_secrets_json["memories"].to_string();

        // API Key must already be delegated to the vessel
        let claude_result = match call_anthropic(
            &anthropic_query.prompt,
            &secret_context,
            anthropic_query.tools,
            anthropic_query.stream.unwrap_or(false),
            &mut metrics
        ).await {
            Ok(result) => {
                info!("\n{} {}\n", "Claude:".bright_black(), result.text.yellow());
                serde_json::to_value(result).ok()
            },
            Err(e) => {
                warn!("Failed to call Anthropic API: {}", e);
                None
            }
        };

        let deepseek_result: Option<serde_json::Value> = None;
        // // 2. Test DeepSeek API if key is available
        // let deepseek_result = if let Some(deepseek_api_key) = memory_secrets_json["deepseek_api_key"].as_str() {
        //     info!("Decrypted DeepSeek API key, querying DeepSeek...");
        //     metrics.record_attempt();

        //     match call_deepseek(
        //         &prompt,
        //         &memory_secrets_json["memories"].to_string(),
        //         &mut metrics
        //     ).await {
        //         Ok(result) => {
        //             info!("\n{} {}\n", "DeepSeek:".bright_black(), result.text.green());
        //             serde_json::to_value(result).ok()
        //         },
        //         Err(e) => {
        //             warn!("Failed to call DeepSeek API: {}", e);
        //             None
        //         }
        //     }
        // } else {
        //     None
        // };

        let usage_report = metrics.generate_report();
        println!("{}", usage_report);

        Ok(ExecuteWithMemoryReverieResult {
            claude: claude_result,
            deepseek: deepseek_result,
            usage_report: metrics
        })
    }
}


#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct AnthropicQuery {
    pub prompt: String,
    pub tools: Option<serde_json::Value>,
    pub stream: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecuteWithMemoryReverieResult {
    pub claude: Option<serde_json::Value>,
    pub deepseek: Option<serde_json::Value>,
    pub usage_report: MCPToolUsageMetrics,
}

