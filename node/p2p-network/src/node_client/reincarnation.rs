
use color_eyre::{Result, eyre::anyhow};
use libp2p::PeerId;
use tracing::{info, debug, error, warn};

use colored::Colorize;
use crate::network_events::peer_manager::peer_info::AgentVesselInfo;
use crate::network_events::NodeIdentity;
use crate::types::{
    AgentNameWithNonce,
    CapsuleFragmentMessage,
    GossipTopic,
    NetworkEvent,
    NextTopic,
    PrevTopic,
    RespawnId,
    TopicSwitch,
    NodeVesselWithStatus,
    Reverie,
    ReverieId,
};
use crate::behaviour::heartbeat_behaviour::TeePayloadOutEvent;
use runtime::reencrypt::{UmbralKey, VerifiedCapsuleFrag};
use runtime::llm::{test_claude_query, AgentSecretsJson};

use super::commands::NodeCommand;
use super::NodeClient;




impl<'a> NodeClient<'a> {

   pub(super) async fn handle_respawn_request(
        &mut self,
        agent_name_nonce: AgentNameWithNonce,
        total_frags: usize,
        next_vessel_peer_id: PeerId,
        current_vessel_peer_id: PeerId,
    ) -> Result<()> {

        info!("\nHandle Respawn Request: {:?}", agent_name_nonce);
        info!("total_frags: {:?}", total_frags);
        info!("next_vessel_peer_id: {:?}", next_vessel_peer_id);
        info!("current_vessel_peer_id: {:?}", current_vessel_peer_id);

        let prev_agent = agent_name_nonce.clone();
        let next_agent = agent_name_nonce.make_next_agent();
        let next_nonce = next_agent.nonce();

        let mut agent_secrets_json = self.request_respawn_cfrags(
            &prev_agent,
            current_vessel_peer_id
        ).await?;

        agent_secrets_json.agent_nonce = next_nonce;
        self.agent_secrets_json = Some(agent_secrets_json.clone());

        // TODO: fix hardcoded threshold
        let threshold = 2;

        // // subscribe to all new agent_nonce channels to broadcast
        // let next_topics = (0..total_frags).into_iter()
        //     .map(|frag_num| {
        //         GossipTopic::BroadcastKfrag(
        //             next_agent.clone(),
        //             total_frags,
        //             frag_num
        //         ).to_string()
        //     })
        //     .collect::<Vec<String>>();

        // let topic_switch = TopicSwitch {
        //     next_topic: NextTopic {
        //         agent_name_nonce: next_agent.clone(),
        //         threshold,
        //         total_frags,
        //     },
        //     prev_topic: Some(PrevTopic {
        //         agent_name_nonce: agent_name_nonce.clone(),
        //         peer_id: Some(current_vessel_peer_id)
        //     }),
        // };
        // let num_subscribed = self.broadcast_switch_topic_nc(topic_switch).await;
        // info!("num peers subscribed: {:?}", num_subscribed);


        // Test LLM API key from decrypted Reverie works
        if let Some(_anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
            info!("Decrypted LLM API keys, querying LLM (paused)");
            // let response = test_claude_query(
            //     _anthropic_api_key,
            //     "What is your name and what do you do?",
            //     &agent_secrets_json.context
            // ).await.unwrap();
            // info!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
        }

        let reverie = self.create_reverie(
            agent_secrets_json.clone(),
            threshold,
            total_frags
        )?;
        // TODO: post-respawn checks:
        // 1. test LLM API works
        // 2. re-encrypt secrets + provide TEE attestation of it
        // 3. confirm shutdown of old vessel
        // 4. then broadcast topic switch to new Agent with new nonce

        let (reverie, target_vessel) = self.broadcast_kfrags2(
            reverie.id.clone(),
            Some(next_agent),
        ).await?;

        info!("{}", format!("Respawn Request complete.\n\n").green());
        Ok(())
    }

    // This needs to happen within the TEE as there is a decryption step, and the original
    // plaintext is revealed within the TEE, before being re-encrypted under PRE again.
    pub async fn request_respawn_cfrags(
        &mut self,
        agent_name_nonce: &AgentNameWithNonce,
        current_vessel_peer_id: PeerId
    ) -> Result<AgentSecretsJson> {

        let reverie_id = match self.get_reverie_id_from_agent_name(agent_name_nonce).await {
            None => return Err(anyhow!("No reverie_id found for agent name nonce: {:?}", agent_name_nonce)),
            Some(reverie_id) => reverie_id,
        };

        let cfrags_raw = self.request_cfrags(
            reverie_id.clone(),
            current_vessel_peer_id
        ).await;


        let reverie = self.get_reverie(reverie_id.clone()).await?;
        let capsule = serde_json::from_slice::<umbral_pre::Capsule>(&reverie.reverie.umbral_capsule)?;

        let (
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw, capsule.clone());

        let next_agent_secrets = self.decrypt_cfrags(
            reverie,
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        );

        next_agent_secrets
    }

    /// Client sends AgentSecretsJson over TLS or some secure channel.
    /// Node encrypts with PRE and broadcasts fragments to the network
    pub async fn spawn_agent(
        &mut self,
        agent_secrets: AgentSecretsJson,
        threshold: usize,
        total_frags: usize,
    ) -> Result<NodeVesselWithStatus> {

        assert!(threshold <= total_frags, "Threshold must be less than or equal to total fragments");

        // Encrypt using PRE
        let reverie = self.create_reverie(agent_secrets.clone(), threshold, total_frags)?;
        // prove node has decrypted AgentSecret
        // prove node has TEE attestation
        // prove node has re-encrypted AgentSecret

        // Create a unique ID for a "Reverie"––a secret memory that alters how a Host behaves
        let agent_name_nonce = AgentNameWithNonce(
            agent_secrets.agent_name,
            agent_secrets.agent_nonce
        );

        let (reverie, target_vessel) = self.broadcast_kfrags2(
            reverie.id.clone(),
            Some(agent_name_nonce),
        ).await?;

        info!("RequestResponse broadcast of kfrags complete.");

        Ok(target_vessel)
    }

    pub async fn get_reverie_id_from_agent_name(
        &self,
        agent_name_nonce: &AgentNameWithNonce
    ) -> Option<ReverieId> {
        let (sender, receiver) = tokio::sync::oneshot::channel();

        self.command_sender
            .send(NodeCommand::GetAgentReverieId {
                agent_name_nonce: agent_name_nonce.clone(),
                sender: sender,
            })
            .await.expect("Command receiver not to bee dropped");

        receiver.await.expect("get reverie receiver not to drop")
    }

    pub async fn broadcast_switch_topic_nc(&mut self, topic_switch: TopicSwitch) -> Result<usize> {
        // prove node has decrypted AgentSecret
        // prove node has TEE attestation
        // prove node has re-encrypted AgentSecret
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.command_sender.send(NodeCommand::BroadcastSwitchTopic(
            topic_switch,
            sender
        )).await.expect("Command receiver not to be dropped");

        receiver.await.map_err(|e| e.into())
    }
}
