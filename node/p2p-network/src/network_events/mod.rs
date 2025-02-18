mod chat;
mod swarm_handlers;
mod command_handlers;
mod gossipsub_handlers;
mod kademlia_handlers;
mod request_response_handlers;
mod query_node_state;
pub(crate) mod peer_manager;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use color_eyre::Result;
use colored::Colorize;
use futures::StreamExt;
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{info, warn, debug};

use crate::{
    SendError,
    get_node_name,
};
use crate::behaviour::heartbeat_behaviour::HeartbeatConfig;
use crate::node_client::NodeCommand;
use crate::types::{
    AgentNameWithNonce,
    ChatMessage,
    FragmentNumber,
    GossipTopic,
    NetworkEvent,
    RespawnId,
    TopicSwitch,
    UmbralPeerId,
    UmbralPublicKeyResponse
};
use crate::node_client::container_manager::{ContainerManager, RestartReason};
use crate::create_network::NODE_SEED_NUM;
use crate::behaviour::Behaviour;
use crate::network_events::peer_manager::AgentVesselTransferInfo;
use runtime::reencrypt::UmbralKey;
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
// use runtime::llm::test_claude_query;

pub struct NetworkEvents<'a> {
    swarm: Swarm<Behaviour>,
    seed: usize,
    peer_id: PeerId, // This node's PeerId
    node_name: &'a str,
    // Umbral fragments
    umbral_key: UmbralKey,

    command_receiver: mpsc::Receiver<NodeCommand>,
    chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
    network_event_sender: mpsc::Sender<NetworkEvent>,
    // tracks own heartbeat status
    internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,

    // tracks peer heartbeats status
    peer_heartbeat_checker: time::Interval,
    peer_manager: PeerManager<'a>,

    // pending P2p network requests
    pending: PendingRequests,
    topics: HashMap<String, IdentTopic>,

    container_manager: Arc<RwLock<ContainerManager>>
}

struct PendingRequests {
    get_providers: HashMap<
        kad::QueryId,
        oneshot::Sender<HashSet<PeerId>>
    >,
    get_umbral_pks: HashMap<
        UmbralPeerId,
        mpsc::Sender<UmbralPublicKeyResponse>
    >,
    request_fragments: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<Vec<u8>, SendError>>
    >,
    respawns: HashSet<RespawnId>,
}

impl PendingRequests {
    fn new() -> Self {
        Self {
            get_providers: Default::default(),
            get_umbral_pks: Default::default(),
            request_fragments: Default::default(),
            respawns: Default::default(),
        }
    }
}

impl<'a> NetworkEvents<'a> {

    pub fn new(
        seed: usize,
        swarm: Swarm<Behaviour>,
        peer_id: PeerId,
        node_name: &'a str,
        umbral_key: UmbralKey,
        command_receiver: mpsc::Receiver<NodeCommand>,
        chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
        network_event_sender: mpsc::Sender<NetworkEvent>,
        internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,
        container_manager: Arc<RwLock<ContainerManager>>
    ) -> Self {

        Self {
            seed,
            swarm,
            peer_id,
            node_name: &node_name,
            umbral_key: umbral_key,
            command_receiver,
            chat_cmd_receiver,
            network_event_sender,
            internal_heartbeat_fail_receiver,
            peer_heartbeat_checker: tokio::time::interval(Duration::from_secs(1)),
            peer_manager: PeerManager::new(&node_name, peer_id),
            pending: PendingRequests::new(),
            topics: HashMap::new(),
            container_manager,
        }
    }

    fn nname(&self) -> String {
        format!("{}{}", self.node_name.yellow(), ">".blue())
    }

    pub async fn listen_for_network_events(mut self) {
        loop {
            tokio::select! {
                _ = self.peer_heartbeat_checker.tick() => {

                    self.swarm.behaviour_mut().heartbeat.increment_block_height();
                    // Replace interval/block_height with a consensus (BFT) and
                    // a runtime (WASM or REVM) that tracks block height,
                    // order of transactions, state roots from executing state transitions
                    self.handle_peer_heartbeat_failure().await
                        .expect("error handling heartbeat failure");
                }
                swarm_event = self.swarm.select_next_some() => {
                    self.handle_swarm_event(swarm_event).await;
                },
                hb = self.internal_heartbeat_fail_receiver.recv() => match hb {
                    // internal Heartbeat failure
                    Some(hb_config) => self.handle_internal_heartbeat_failure(hb_config).await,
                    None => break // channel closed, shutting down the network event loop.
                },
                command = self.command_receiver.recv() => match command {
                    Some(c) => self.handle_command(c).await,
                    None => return
                },
                chat_message = self.chat_cmd_receiver.recv() => match chat_message {
                    Some(cm) => self.broadcast_chat_message(cm).await,
                    None => return
                },
            }
        }
    }

    async fn handle_peer_heartbeat_failure(&mut self) -> Result<()> {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation();

        let peer_ids = self.peer_manager.get_connected_peers()
            .iter()
            .map(|p| format!("{}", get_node_name(p.0)))
            .collect::<Vec<String>>();

        info!("{} connected peers: {:?}", self.nname(), peer_ids);

        let mut failed_agents_placeholder = vec![];
        let peer_info = self.peer_manager.peer_info.clone();

        // iterate and check which peers have stopped sending heartbeats
        for (peer_id, peer_info) in peer_info.iter() {

            let duration = peer_info.heartbeat_data.duration_since_last_heartbeat();
            let node_name = get_node_name(&peer_id).magenta();
            println!("{}\tlast seen {:.2?} seconds ago", node_name, duration);

            if duration > max_time_before_respawn {

                if let None = &peer_info.agent_vessel {
                    println!("{} failed but wasn't hosting an agent.", node_name);
                    // TODO remove peer kfrags, will disconnect automatically after timeout
                    // self.remove_peer(peer_id);
                }

                if let Some(AgentVesselTransferInfo {
                    agent_name_nonce: prev_agent,
                    total_frags,
                    next_vessel_peer_id,
                    prev_vessel_peer_id
                }) = &peer_info.agent_vessel {

                    let next_agent = AgentNameWithNonce(prev_agent.0.clone(), prev_agent.1 + 1);
                    info!("{}", format!("{} failed. Consensus: voting to reincarnate agent '{}'",
                        node_name,
                        prev_agent
                    ).magenta());

                    // TODO: consensus mechanism to vote for reincarnation
                    failed_agents_placeholder.push(prev_agent);

                    // mark as respawning...
                    let respawn_id = RespawnId::new(prev_agent, prev_vessel_peer_id);
                    self.pending.respawns.insert(respawn_id.clone());

                    ////////////////////////////////////////////////
                    // If this node is the next vessel for the agent
                    if self.peer_id == *next_vessel_peer_id {

                        // // subscribe to all new agent_nonce channels to broadcast
                        // let new_topics = vec![
                        //     GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 0).to_string(),
                        //     GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 1).to_string(),
                        //     GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 2).to_string(),
                        //     GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 3).to_string(),
                        // ];
                        // self.subscribe_topics(&new_topics);

                        // Dispatch a Respawn(agent_name) event to NetworkEvents
                        // - regenerates PRE ciphertexts, kfrags, and reencryption keys
                        // - re-broadcasts the new PRE fragments to peers
                        let _ = self.network_event_sender
                            .send(NetworkEvent::RespawnRequest(
                                AgentVesselTransferInfo {
                                    agent_name_nonce: prev_agent.clone(),
                                    total_frags: *total_frags,
                                    next_vessel_peer_id: *next_vessel_peer_id,
                                    prev_vessel_peer_id: prev_vessel_peer_id.clone()
                                }
                            )).await;

                        info!("{} '{}' {} {}", "Respawning agent".blue(),
                            prev_agent.to_string().green(),
                            "in new vessel:".blue(),
                            self.node_name.yellow()
                        );

                        // 1) remove Vessel from PeerManagers and Swarm
                        self.remove_peer(peer_id);

                        // 2) broadcast peers, tell peers to update their PeerManager fields as well:
                        // - kfrags_peers
                        // - peers_to_agent_frags
                        // - peer_info

                    } else {

                        // If node is not the next vessel:
                        // Subscribe to next agent_nonce channel and wait for when it
                        // is reincarnated and broadcasting cfrags.

                        // TODO: once respawn is finished,
                        // 1) new Vessel will tell other nodes to update PeerManager and
                        // 2) update pending.respawns:
                        // self.pending.respawns.remove(&respawn_id_result);

                        if self.pending.respawns.contains(&respawn_id) {
                            info!("Respawn pending: {} -> {}", next_agent, self.node_name);

                        } else {

                            let frag_num = self.seed % total_frags;
                            info!("Not the next vessel. Subscribing to next agent on frag_num({})\n", frag_num);
                            // Subscribe to new agent fragments channel
                            self.subscribe_topics(&vec![
                                GossipTopic::BroadcastKfrag(next_agent, *total_frags, frag_num).to_string(),
                            ]);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_internal_heartbeat_failure(&mut self, heartbeat_config: HeartbeatConfig) {
        // Internal heartbeat failure is when this node fails to send heartbeats to external
        // nodes. It realizes it is no longer connected to the network.
        info!("{}", format!("{:?}", heartbeat_config).red());
        info!("{}", "Initiating recovery...".green());
        info!("{}", "Todo: attempting to broadcast/save last good agent_secrets reencryption fragments.".green());
        info!("{}", "Todo: Delete agent secrets, to prevent duplicated agents.".green());

        let peers: Vec<PeerId> = self.swarm.connected_peers().cloned().collect();
        let total_peers = peers.len();
        info!("{}", format!("Disconnecting from {} peers", total_peers).yellow());
        for peer_id in peers {
            self.swarm.disconnect_peer_id(peer_id).ok();
        }

        let time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation()
            .as_secs();

        // heartbeat protocol failure then triggers ContainerManager to shutdown/reboot container
        for second in 0..time_before_respawn {
            let countdown = time_before_respawn - second;
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("{}", format!("Rebooting container in {} seconds", countdown).yellow());
        };

        self.container_manager
            .write()
            .await
            .trigger_restart(RestartReason::ScheduledHeartbeatFailure)
            .await.ok();

        // TODO: Shutdown LLM runtime (if in Vessel Mode), but continue attempting
        // to broadcast the agent_secrets reencryption fragments and ciphertexts.
        //
        // If the node never reconnects to the network, then 1up-network nodes will
        // form consensus that the Vessel is dead, and begin reincarnating the Agent
        // from it's last public agent_secret ciphertexts on the Kademlia network.
        //
        // In this case, the node should also delete it's Agent secrets to prevent
        // duplicate instances of an agent running simultaneously
    }

    fn remove_peer(&mut self, peer_id: &PeerId) {
        // Remove from PeerManager locally
        self.peer_manager.remove_kfrags_broadcast_peer(peer_id);
        self.peer_manager.remove_peer_info(peer_id);
        // Remove peer from GossipSub
        self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
        // Remove UmbralPeerId of the Peer on Kademlia
        self.swarm
            .behaviour_mut()
            .kademlia
            .remove_record(&kad::RecordKey::new(&UmbralPeerId::from(peer_id).to_string()));
    }

    fn save_peer(&mut self,
        sender_peer_id: &PeerId,
        agent_name_nonce: AgentNameWithNonce,
        frag_num: FragmentNumber
    ) {
        // Add to PeerManager locally
        self.peer_manager.insert_peer_agent_fragments(&sender_peer_id, &agent_name_nonce, frag_num);
        self.peer_manager.insert_kfrags_broadcast_peer(sender_peer_id.clone(), &agent_name_nonce, frag_num);
        self.peer_manager.insert_peer_info(sender_peer_id.clone());
    }

    pub(crate) async fn simulate_heartbeat_failure(&mut self) {
        self.swarm.behaviour_mut()
            .heartbeat
            .trigger_heartbeat_failure().await;
    }

}
