use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use libp2p::PeerId;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use tracing::{info, debug, error, warn};

use crate::network_events::NodeIdentity;
use crate::types::{
    ReverieNameWithNonce,
    NetworkEvent,
    RespawnId,
    NodeKeysWithVesselStatus,
    Reverie,
    ReverieId,
    ReverieType,
    SignatureType,
    VerifyingKey,
};
use crate::behaviour::heartbeat_behaviour::TeePayloadOutEvent;
use runtime::reencrypt::{UmbralKey, VerifiedCapsuleFrag};
use runtime::llm::{test_claude_query};

use super::commands::NodeCommand;
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

        self.broadcast_reverie_keyfrags(&reverie, target_vessel.peer_id, target_kfrag_providers).await?;

        info!("RequestResponse broadcast of kfrags complete.");
        Ok(reverie)
    }

    // This needs to happen within the TEE as there is a decryption step, and the original
    // plaintext is revealed within the TEE, before being re-encrypted under PRE again.
    async fn reconstruct_memory_cfrags<T: Serialize + DeserializeOwned>(
        &mut self,
        reverie_id: ReverieId,
        reverie_type: ReverieType,
        prev_failed_vessel_peer_id: libp2p::PeerId,
        signature: SignatureType
    ) -> Result<T> {

        let reverie_msg = self.get_reverie(reverie_id.clone(), reverie_type).await?;
        let capsule = reverie_msg.reverie.encode_capsule()?;

        let cfrags_raw = self.request_cfrags(
            reverie_id.clone(),
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

        println!("memory_secrets_json: {:?}", memory_secrets_json);

        // Then execute LLM with Reverie as context:
        // 1. Test LLM API key from decrypted Reverie works
        if let Some(_anthropic_api_key) = memory_secrets_json["anthropic_api_key"].as_str() {
            info!("Decrypted LLM API keys, querying LLM (paused)");
            // let response = test_claude_query(
            //     _anthropic_api_key,
            //     "What is your name and what do you do?",
            //     &agent_secrets_json.context
            // ).await.unwrap();
            // info!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
        }

        Ok(())
    }

}
