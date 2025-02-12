mod chat;
mod swarm_handlers;
mod command_handlers;
mod gossipsub_handlers;
mod kademlia_handlers;
mod request_response_handlers;
pub(crate) mod peer_manager;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use color_eyre::Result;
use colored::Colorize;
use futures::StreamExt;
use tokio::sync::{RwLock, mpsc, oneshot};
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use runtime::reencrypt::UmbralKey;
use crate::{
    SendError,
    get_node_name,
    short_peer_id
};
use crate::behaviour::heartbeat_behaviour::HeartbeatConfig;
use crate::node_client::NodeCommand;
use crate::types::{
    AgentNameWithNonce,
    ChatMessage,
    FragmentNumber,
    GossipTopic,
    NetworkEvent,
    TopicSwitch,
    UmbralPeerId,
    UmbralPublicKeyResponse
};
use crate::node_client::container_manager::{ContainerManager, RestartReason};
use crate::create_network::NODE_SEED_NUM;
use crate::behaviour::Behaviour;
use crate::network_events::peer_manager::AgentVesselTransferInfo;
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
use runtime::llm::{AgentSecretsJson, test_claude_query};

pub struct NetworkEvents<'a> {
    seed: usize,
    swarm: Swarm<Behaviour>,
    peer_id: PeerId, // This node's PeerId
    node_name: &'a str,
    command_receiver: mpsc::Receiver<NodeCommand>,
    chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
    network_event_sender: mpsc::Sender<NetworkEvent>,

    // Umbral fragments
    umbral_key: UmbralKey,

    internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,

    interval: time::Interval,

    // tracks peer heartbeats status
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
    respawns: HashSet<(AgentNameWithNonce, PeerId)>,
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
        command_receiver: mpsc::Receiver<NodeCommand>,
        chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
        network_event_sender: mpsc::Sender<NetworkEvent>,
        umbral_key: UmbralKey,
        internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,
        container_manager: Arc<RwLock<ContainerManager>>
    ) -> Self {

        Self {
            seed,
            swarm,
            peer_id,
            node_name: &node_name,
            command_receiver,
            chat_cmd_receiver,
            network_event_sender,
            umbral_key: umbral_key,
            internal_heartbeat_fail_receiver,
            interval: tokio::time::interval(Duration::from_secs(1)),
            peer_manager: PeerManager::new(&node_name),
            pending: PendingRequests::new(),
            topics: HashMap::new(),
            container_manager,
        }
    }

    fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "NetworkEvents".blue(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn listen_for_network_events(mut self) {
        loop {
            tokio::select! {
                _instant = self.interval.tick() => {

                    self.swarm.behaviour_mut().heartbeat.increment_block_height();
                    // Replace interval/block_height with a consensus (BFT) and
                    // a runtime (WASM or REVM) that tracks block height,
                    // order of transactions, state roots from executing state transitions

                    self.handle_peer_heartbeat_failure().await;
                }
                swarm_event = self.swarm.select_next_some() => {
                    self.handle_swarm_event(swarm_event).await;
                },
                heartbeat_fail = self.internal_heartbeat_fail_receiver.recv() => match heartbeat_fail {
                    // internal Heartbeat failure
                    Some(hb) => self.handle_internal_heartbeat_failure(hb).await,
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

    async fn handle_peer_heartbeat_failure(&mut self) {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation();

        let peer_ids = self.peer_manager.get_connected_peers()
            .iter().map(|p| format!("{}", get_node_name(p.0)))
            .collect::<Vec<String>>();

        println!("Connected peers: {:?}", peer_ids);

        let mut failed_agents = vec![];
        let topic_switch: Option<TopicSwitch> = None;

        // iterate and check which peers have stopped sending heartbeats
        for (peer_id, peer_info) in self.peer_manager.peer_info.clone().iter() {

            let duration = peer_info.heartbeat_data.duration_since_last_heartbeat();
            let node_name = get_node_name(&peer_id).magenta();
            println!("{}\tlast seen {:.2?} seconds ago", node_name, duration);

            if duration > max_time_before_respawn {

                if let None = &peer_info.agent_vessel {
                    println!("{} failed but wasn't hosting an agent.", node_name);
                    // TODO
                    // remove peer kfrags, it will disconnect automatically after a while
                    // self.remove_peer(peer_id);
                }

                if let Some(AgentVesselTransferInfo {
                    agent_name_nonce: prev_agent,
                    total_frags,
                    next_vessel_peer_id,
                    prev_vessel_peer_id
                }) = &peer_info.agent_vessel {

                    let next_agent = AgentNameWithNonce(prev_agent.0.clone(), prev_agent.1 + 1);
                    let respawn_pending = self.pending.respawns.contains(&(prev_agent.clone(), *next_vessel_peer_id));

                    if respawn_pending {
                        self.log(format!("Respawn pending: {} -> {}", next_agent, self.node_name));

                    } else {

                        self.log(format!(
                            "{} failed. Consensus: voting to reincarnate agent '{}'",
                            node_name,
                            prev_agent
                        ).magenta());

                        // TODO: consensus mechanism to vote for reincarnation
                        failed_agents.push(prev_agent);

                        // if this node is the next vessel for the agent
                        if self.peer_id == *next_vessel_peer_id {

                            // subscribe to all new agent_nonce channels to broadcast
                            self.subscribe_topics(&vec![
                                GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 0).to_string(),
                                GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 1).to_string(),
                                GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 2).to_string(),
                                GossipTopic::BroadcastKfrag(next_agent.clone(), *total_frags, 3).to_string(),
                            ]);

                            // Dispatch a Respawn(agent_name) event to NetworkEvents
                            let _ = self.network_event_sender
                                .send(NetworkEvent::RespawnRequiredRequest(
                                    AgentVesselTransferInfo {
                                        agent_name_nonce: prev_agent.clone(),
                                        total_frags: *total_frags,
                                        next_vessel_peer_id: *next_vessel_peer_id,
                                        prev_vessel_peer_id: prev_vessel_peer_id.clone()
                                    }
                                )).await;

                            self.log(format!("{} '{}' {} {}", "Respawning agent".blue(),
                                prev_agent.to_string().green(),
                                "in new vessel:".blue(),
                                self.node_name.yellow()
                            ));

                            // mark as respawning...
                            self.pending.respawns.insert((prev_agent.clone(), *next_vessel_peer_id));

                            // TODO: once respawn is pending,
                            // 1) unsubscribe from Topic + subscribe to new Topic
                            // 2) remove Vessel from PeerManagers
                            // 3) broadcast peers to tell them to do the same


                            // topic_switch = Some(TopicSwitch {
                            //     prev_topic: agent_name.to_string(), //
                            //     next_topic: agent_name.to_string(), // +1 increment AgentName nonce
                            //     prev_vessel_peer_id: prev_vessel_peer_id.clone()
                            // });

                            // let _ = self.network_event_sender
                            //     .send(NetworkEvent::ReBroadcastKfrags(agent_name.to_owned())).await;
                            // TODO: once broadcast happens, tell peers to update their PeerManager fields
                            // - kfrags_peers
                            // - peers_to_agent_frags
                            // - peer_info

                        } else {

                            // If node is not the next vessel:
                            // Subscribe to next agent_nonce channel for when it
                            // is reincarnated and broadcasting cfrags
                            let frag_num = self.seed % total_frags;
                            self.log(format!("\nNext frag_num: {}\n", frag_num));

                            // Broadcast new agent fragments
                            self.subscribe_topics(&vec![
                                GossipTopic::BroadcastKfrag(next_agent, *total_frags, frag_num).to_string(),
                            ]);

                            // mark as respawning...
                            self.pending.respawns.insert((prev_agent.clone(), *next_vessel_peer_id));
                        }

                        // TODO
                        // remove peer kfrags, it will disconnect automatically after a while
                        self.remove_peer(peer_id);
                        // self.peer_manager.remove_kfrags_peers_by_agent_name(agent_name, *agent_nonce);
                    }
                }
            }
        };

        if let Some(topic_switch2) = topic_switch {
            self.log(format!("Broadcasting topic switch"));
            self.broadcast_topic_switch(&topic_switch2).await;
        }
    }

    async fn handle_internal_heartbeat_failure(&mut self, heartbeat_config: HeartbeatConfig) {
        // Internal heartbeat failure is when this node fails to send heartbeats to external
        // nodes. It realizes it is no longer connected to the network.
        self.log(format!("{:?}", heartbeat_config).red());
        self.log(format!("Initiating recovery...").green());
        self.log(format!("Todo: attempting to broadcast/save last good agent_secrets reencryption fragments.").green());
        self.log(format!("Todo: Delete agent secrets, to prevent duplicated agents.").green());

        for peer_id in self.swarm.connected_peers() {
            //
            // self.swarm.behaviour_mut().send_event(ToSwarm::CloseConnection {
            //     peer_id: *peer_id,
            //     connection: ConnectionId::new_unchecked(0), // You might need to handle actual connection IDs
            // });
        }

        self.log(format!("Shutting down container housing LLM runtime.").green());
        // heartbeat protocol failure then triggers ContainerManager to shutdown/reboot container
        self.container_manager
            .write()
            .await
            .trigger_restart(RestartReason::ScheduledHeartbeatFailure)
            .await.ok();

        // TODO
        // Shutdown the LLM runtime (if in Vessel Mode), but continue attempting
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
