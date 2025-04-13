
use color_eyre::{Result, eyre::anyhow};
use libp2p::PeerId;
use tracing::{info, debug, error, warn};

use colored::Colorize;
use crate::network_events::NodeIdentity;
use crate::types::{
    ReverieNameWithNonce,
    NetworkEvent,
    RespawnId,
    NodeVesselWithStatus,
    Reverie,
    ReverieId,
    ReverieType,
    AgentVesselInfo,
};
use crate::behaviour::heartbeat_behaviour::TeePayloadOutEvent;
use runtime::reencrypt::{UmbralKey, VerifiedCapsuleFrag};
use runtime::llm::{test_claude_query, AgentSecretsJson};

use super::commands::NodeCommand;
use super::NodeClient;



impl<'a> NodeClient<'a> {

    /// Client sends AgentSecretsJson over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn spawn_agent(
        &mut self,
        agent_secrets: AgentSecretsJson,
        threshold: usize,
        total_frags: usize,
    ) -> Result<NodeVesselWithStatus> {

        if threshold > total_frags {
            return Err(anyhow!("Threshold must be less than or equal to total fragments"));
        }

        let agent_name_nonce = ReverieNameWithNonce(
            agent_secrets.agent_name.clone(),
            agent_secrets.agent_nonce.clone()
        );

        // Create a "Reverie"––an encrypted memory that alters how a Host behaves
        let reverie = self.create_reverie(
            agent_secrets,
            ReverieType::SovereignAgent(agent_name_nonce.clone()),
            threshold,
            total_frags
        )?;

        let (
            reverie,
            target_vessel
        ) = self.broadcast_reverie_keyfrags(reverie.id.clone()).await?;

        info!("RequestResponse broadcast of kfrags complete.");

        Ok(target_vessel)
    }

    pub async fn get_reverie_id_from_agent_name(&self, agent_name_nonce: &ReverieNameWithNonce) -> Option<ReverieId> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.command_sender
            .send(NodeCommand::GetReverieIdFromAgentName {
                agent_name_nonce: agent_name_nonce.clone(),
                sender: sender,
            })
            .await.expect("Command receiver not to bee dropped");

        receiver.await.expect("get reverie receiver not to drop")
    }

    pub(super) async fn handle_respawn_request(
        &mut self,
        prev_reverie_id: ReverieId,
        prev_reverie_type: ReverieType,
        threshold: usize,
        total_frags: usize,
        next_vessel_peer_id: PeerId,    // This node is the next vessel
        prev_failed_vessel_peer_id: PeerId, // Previous (failed) vessel
    ) -> Result<()> {

        let prev_agent_name_nonce = match &prev_reverie_type {
            ReverieType::SovereignAgent(agent_name_nonce) => agent_name_nonce.clone(),
            _ => return Err(anyhow!("Previous Reverie is not a SovereignAgent")),
        };
        info!("\nHandle respawn request: {:?}", prev_agent_name_nonce);
        info!("total_frags: {:?}", total_frags);
        info!("next_vessel_peer_id: {:?}", next_vessel_peer_id);
        info!("prev_failed_vessel_peer_id: {:?}", prev_failed_vessel_peer_id);

        let prev_agent = prev_agent_name_nonce.clone();
        let next_agent = prev_agent_name_nonce.increment_nonce();
        let next_nonce = next_agent.nonce();

        let mut agent_secrets_json = self.request_respawn_cfrags(
            prev_reverie_id.clone(),
            prev_reverie_type,
            prev_failed_vessel_peer_id
        ).await?;

        agent_secrets_json.agent_nonce = next_nonce;
        self.agent_secrets_json = Some(agent_secrets_json.clone());

        // Respawn checks:
        // 1. test LLM API works
        // 2. re-encrypt secrets + provide TEE attestation of it
        // 3. mark respawn complete / old vessel died

        // 1. Test LLM API key from decrypted Reverie works
        if let Some(_anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
            info!("Decrypted LLM API keys, querying LLM (paused)");
            // let response = test_claude_query(
            //     _anthropic_api_key,
            //     "What is your name and what do you do?",
            //     &agent_secrets_json.context
            // ).await.unwrap();
            // info!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
        }

        // 2. re-encrypt secrets + provide TEE attestation of it
        let reverie = self.create_reverie(
            agent_secrets_json.clone(),
            ReverieType::SovereignAgent(next_agent.clone()),
            threshold,
            total_frags
        )?;

        let (
            reverie,
            target_vessel
        ) = self.broadcast_reverie_keyfrags(reverie.id.clone()).await?;

        // 3. mark respawn complete / old vessel died
        self.command_sender.send(NodeCommand::MarkPendingRespawnComplete {
            prev_reverie_id: prev_reverie_id.clone(),
            prev_peer_id: prev_failed_vessel_peer_id,
            prev_agent_name_nonce: prev_agent,
        }).await.ok();

        info!("{}", format!("Respawn Request complete.\n\n").green());
        Ok(())
    }

    // This needs to happen within the TEE as there is a decryption step, and the original
    // plaintext is revealed within the TEE, before being re-encrypted under PRE again.
    pub async fn request_respawn_cfrags(
        &mut self,
        prev_reverie_id: ReverieId,
        prev_reverie_type: ReverieType,
        prev_failed_vessel_peer_id: PeerId
    ) -> Result<AgentSecretsJson> {

        let reverie_msg = self.get_reverie(prev_reverie_id.clone(), prev_reverie_type).await?;
        let capsule = reverie_msg.reverie.encode_capsule()?;

        let cfrags_raw = self.request_cfrags(
            prev_reverie_id.clone(),
            prev_failed_vessel_peer_id
        ).await;

        let (
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw, capsule.clone())?;

        let next_agent_secrets = self.decrypt_cfrags(
            reverie_msg,
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        );

        next_agent_secrets
    }

}
