
use color_eyre::{Result, eyre::anyhow};
use libp2p::PeerId;
use tracing::{info, debug, error, warn};

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

        let prev_agent = agent_name_nonce.clone();
        let next_agent = agent_name_nonce.make_next_agent();
        let next_nonce = next_agent.nonce();

        let mut agent_secrets_json = self.request_respawn_cfrags(
            &prev_agent,
            current_vessel_peer_id
        ).await?;

        agent_secrets_json.agent_nonce = next_nonce;
        self.agent_secrets_json = Some(agent_secrets_json.clone());

        // subscribe to all new agent_nonce channels to broadcast
        let next_topics = (0..total_frags).into_iter()
            .map(|frag_num| {
                GossipTopic::BroadcastKfrag(
                    next_agent.clone(),
                    total_frags,
                    frag_num
                ).to_string()
            })
            .collect::<Vec<String>>();

        self.subscribe_topics(next_topics.clone()).await?;

        // TODO: fix hardcoded threshold
        let threshold = 2;

        let topic_switch = TopicSwitch {
            next_topic: NextTopic {
                agent_name_nonce: next_agent.clone(),
                total_frags,
                threshold
            },
            prev_topic: Some(PrevTopic {
                agent_name_nonce: agent_name_nonce.clone(),
                peer_id: Some(current_vessel_peer_id)
            }),
        };
        let num_subscribed = self.broadcast_switch_topic_nc(topic_switch).await;
        info!("num peers subscribed: {:?}", num_subscribed);

        let reverie = self.create_reverie(agent_secrets_json.clone())?;
        // TODO: post-respawn checks:
        // 1. test LLM API works
        // 2. re-encrypt secrets + provide TEE attestation of it
        // 3. confirm shutdown of old vessel
        // 4. then broadcast topic switch to new Agent with new nonce

        if let Some(_anthropic_api_key) = agent_secrets_json.anthropic_api_key.clone() {
            info!("Decrypted LLM API keys, querying LLM (paused)");
            // let response = test_claude_query(
            //     _anthropic_api_key,
            //     "What is your name and what do you do?",
            //     &agent_secrets_json.context
            // ).await.unwrap();
            // info!("\n{} {}\n", "Claude:".bright_black(), response.yellow());
        }

        let next_vessel = self.get_next_vessel().await?;
        // randomly choose next vessel to host reverie

        self.broadcast_kfrags(
            reverie.id.clone(),
            next_agent,
            total_frags,
            threshold,
            next_vessel, // no target vessel, randomly choose next vessel
        ).await?;

        self.unsubscribe_topics(next_topics).await?;

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
            reverie_id,
            current_vessel_peer_id
        ).await;

        let (
            verified_cfrags,
            new_vessel_cfrags,
            total_frags_received
        ) = self.parse_cfrags(cfrags_raw);

        let next_agent_secrets = self.decrypt_cfrags(
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
        total_frags: usize,
        threshold: usize,
    ) -> Result<NodeVesselWithStatus> {

        // Encrypt using PRE
        let reverie = self.create_reverie(agent_secrets.clone())?;
        // prove node has decrypted AgentSecret
        // prove node has TEE attestation
        // prove node has re-encrypted AgentSecret

        // Create a unique ID for a "Reverie"––a secret memory that alters how a Host behaves
        let agent_name_nonce = AgentNameWithNonce(agent_secrets.agent_name, agent_secrets.agent_nonce);

        // first check that the broadcaster is subscribed to all fragment channels for the agent
        let topics = (0..total_frags).map(|n| {
            GossipTopic::BroadcastKfrag(
                agent_name_nonce.clone(),
                total_frags,
                n
            ).to_string()
        }).collect::<Vec<String>>();

        // Subscribe to broadcast kfrag topics temporarily
        self.subscribe_topics(topics.clone()).await.ok();
        // Tell other nodes to subscribe to the same kfrag topic
        self.broadcast_switch_topic_nc(TopicSwitch::new(
            agent_name_nonce.clone(),
            total_frags,
            threshold,
            None // no previous agent
        )).await?;

        let next_vessel = self.get_next_vessel().await?;
        // no target vessel, randomly choose next vessel

        let next_vessel_pk = self.broadcast_kfrags(
            reverie.id.clone(),
            agent_name_nonce,
            total_frags,
            threshold,
            next_vessel.clone(),
        ).await?;

        // TODO: should return info:
        // current vessel: peer_id, umbral_public_key, peer_address (info for health monitoring)
        // next vessel: peer_id, umbral_public_key
        // PRE fragment holders
        // PRE total_frags, threshold
        self.unsubscribe_topics(topics).await.ok();
        Ok(next_vessel)
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
