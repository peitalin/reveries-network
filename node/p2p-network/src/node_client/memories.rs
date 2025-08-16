use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tracing::{info, debug, error, warn};
use std::convert::TryFrom;
use hex;
use std::str::FromStr;
use sha3::{Digest, Keccak256};
use runtime::llm::{
    MCPToolUsageMetrics,
    call_anthropic,
    call_deepseek,
};
use runtime::near_runtime::{
    AccessCondition as NearRuntimeAccessCondition,
    ReverieMetadata as NearReverieMetadata,
};
use crate::types::{
    Reverie,
    ReverieId,
    ReverieType,
    AccessCondition as P2PNetworkAccessCondition,
    AccessKey,
};
use crate::env_var::EnvVars;
use crate::usage_db::{UsageDbPool, read_usage_data_for_reverie};
use super::NodeClient;

// ===============================================

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
}

// ===============================================

impl NodeClient {
    /// Client sends some ReverieType over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn spawn_memory_reverie(
        &mut self,
        memory_secrets: serde_json::Value,
        threshold: usize,
        total_frags: usize,
        access_condition: P2PNetworkAccessCondition, // access condition for using the memory
    ) -> Result<Reverie> {

        if threshold > total_frags {
            return Err(anyhow!("Threshold must be less than or equal to total fragments"));
        }

        // get list of target vessel and kfrag provider nodes
        let (
            target_vessel,
            target_kfrag_providers
        ) = self.get_prospect_vessels(false).await?;

        // 1. Create a "Reverie"––an encrypted memory or executable
        let reverie = self.create_reverie(
            memory_secrets,
            ReverieType::Memory,
            threshold,
            total_frags,
            target_vessel.umbral_public_key,
            target_vessel.umbral_verifying_public_key,
            access_condition // access_condition to be checked to request cfrags
        )?;

        // 2a. Write Reverie metadata onchain
        if let P2PNetworkAccessCondition::NearContract(_, _, _) = &reverie.access_condition {
            let env_vars = EnvVars::load();
            let near_access_condition = NearRuntimeAccessCondition::try_from(&reverie.access_condition)?;

            println!(
                "Registering Reverie {} on NEAR contract {} by signer {}",
                reverie.id.clone(),
                env_vars.NEAR.NEAR_CONTRACT_ACCOUNT_ID,
                env_vars.NEAR.NEAR_SIGNER_ACCOUNT_ID
            );

            let reverie_type = reverie.reverie_type.to_string();
            let near_runtime = self.near_runtime.clone();
            let create_reverie_outcome = near_runtime.create_reverie(
                &env_vars.NEAR.NEAR_CONTRACT_ACCOUNT_ID,
                &env_vars.NEAR.NEAR_SIGNER_ACCOUNT_ID,
                &env_vars.NEAR.NEAR_SIGNER_PRIVATE_KEY,
                &reverie.id, // reverie_id
                &reverie_type, // reverie_type
                &reverie.description, // description
                near_access_condition, // access_condition
            ).await?;

            println!("create_reverie NEAR Outcome: {:?}", create_reverie_outcome.status);
            assert!(
                matches!(create_reverie_outcome.status, near_primitives::views::FinalExecutionStatus::SuccessValue(_)),
                "create_reverie NEAR transaction failed: {:?}", create_reverie_outcome.status
            );
        }

        // 2b. Broadcast Reverie keyfrags to the network
        let broadcast_keyfrags_outcome = self.broadcast_reverie_keyfrags(
            &reverie,
            target_vessel.peer_id,
            target_kfrag_providers
        ).await?;

        // let (
        //     create_reverie_outcome,
        //     broadcast_reverie_keyfrags_outcome
        // ) = futures::future::join(
        //     create_reverie_future,
        //     broadcast_keyfrags_future
        // ).await;

        info!("RequestResponse broadcast of kfrags complete: {:?}", broadcast_keyfrags_outcome);

        Ok(reverie)
    }

    // This needs to happen within the TEE as there is a decryption step, and the original
    // plaintext is revealed within the TEE, before executing with it as context
    async fn _reconstruct_memory_reverie<T: Serialize + DeserializeOwned>(
        &mut self,
        reverie_id: &ReverieId,
        reverie_type: ReverieType,
        _prev_failed_vessel_peer_id: libp2p::PeerId,
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
        // Execution node signature required to access the delegated API key
        // In the future, this will be a TEE Quote/Attestation
    ) -> Result<()> {

        let (
            api_keys_json,
            spenders_address // user using the memory
        ) = self._reconstruct_memory_reverie::<serde_json::Value>(
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
            _spenders_access_key
        ) = self._reconstruct_memory_reverie::<serde_json::Value>(
            &reverie_id,
            reverie_type,
            self.node_id.peer_id, // prev_failed_vessel_peer_id
            access_key.clone()
        ).await?;

        // Then execute LLM with Reverie as context:
        // 1. Test LLM API key from decrypted Reverie works
        println!("Decrypted secret memory, querying Claude with private contexts...");
        let node_keypair = &self.node_id.id_keys.clone();
        let secret_context = memory_secrets_json["memories"].to_string();

        // API Key must already be delegated to the vessel
        let claude_result = match call_anthropic(
            &anthropic_query.prompt,
            &secret_context,
            anthropic_query.tools,
            anthropic_query.stream.unwrap_or(false),
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

        // 2. Skip DeepSeek API even if key is available
        let deepseek_result: Option<serde_json::Value> = None;

        Ok(ExecuteWithMemoryReverieResult {
            claude: claude_result,
            deepseek: deepseek_result,
        })
    }
}

