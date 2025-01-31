mod chat;
mod event_handlers;
mod command_handlers;
mod gossipsub_handlers;
pub(crate) mod heartbeat_behaviour;
pub(crate) mod peer_manager;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
};
use color_eyre::{Result, eyre::anyhow};
use colored::Colorize;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use runtime::reencrypt::UmbralKey;
use crate::{node_client::NodeCommand, get_node_name, short_peer_id, AgentName};
use crate::types::{
    ChatMessage,
    GossipTopic,
    UmbralPeerId,
    NetworkLoopEvent,
    CapsuleFragmentIndexed,
    UmbralPublicKeyResponse
};
use crate::behaviour::Behaviour;
use crate::event_loop::peer_manager::AgentVessel;
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
use runtime::llm::{AgentSecretsJson, test_claude_query};

pub struct EventLoop {

    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<NodeCommand>,
    network_event_sender: mpsc::Sender<NetworkLoopEvent>,

    // My node's PeerInfo
    peer_id: PeerId,
    node_name: String,

    // chat
    chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
    topics: HashMap<String, IdentTopic>,

    // Umbral fragments
    umbral_key: UmbralKey,
    cfrags: HashMap<AgentName, CapsuleFragmentIndexed>,

    // tracks peer heartbeats status
    peer_manager: PeerManager,

    // pending P2p network requests
    pending: PendingRequests,

    interval: time::Interval,
    internal_heartbeat_fail_receiver: tokio::sync::mpsc::Receiver<String>,
}

struct PendingRequests {
    get_providers: HashMap<
        kad::QueryId,
        oneshot::Sender<HashSet<PeerId>>
    >,
    get_umbral_pks: HashMap<
        UmbralPeerId,
        tokio::sync::mpsc::Sender<UmbralPublicKeyResponse>
    >,
    request_fragments: HashMap<
        request_response::OutboundRequestId,
        oneshot::Sender<Result<Vec<u8>, Box<dyn Error + Send>>>
    >,
    respawns: HashSet<(AgentName, PeerId)>
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

impl EventLoop {

    pub fn new(
        peer_id: PeerId,
        swarm: Swarm<Behaviour>,
        command_receiver: mpsc::Receiver<NodeCommand>,
        network_event_sender: mpsc::Sender<NetworkLoopEvent>,
        chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
        node_name: String,
        umbral_key: UmbralKey,
        internal_heartbeat_fail_receiver: tokio::sync::mpsc::Receiver<String>,
    ) -> Self {

        Self {
            peer_id,
            swarm,
            command_receiver,
            network_event_sender,
            chat_cmd_receiver,
            node_name,
            topics: HashMap::new(),
            cfrags: HashMap::new(),
            umbral_key: umbral_key,
            peer_manager: PeerManager::new(),
            pending: PendingRequests::new(),
            interval: tokio::time::interval(Duration::from_secs(1)),
            internal_heartbeat_fail_receiver,
        }
    }

    fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "NetworkEvent".green(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn listen_for_commands_and_events(mut self) {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat.config
            .max_time_before_rotation();

        loop {
            tokio::select! {
                _instant = self.interval.tick() => {
                    // Replace interval with consensus that tracks block height,
                    // order of transactions, state roots from executing state transitions
                    // in the REVM
                    self.swarm.behaviour_mut().heartbeat.increment_block_height();

                    let peer_ids = self.peer_manager.get_connected_peers()
                        .iter().map(|p| format!("{}", get_node_name(p.0)))
                        .collect::<Vec<String>>();

                    // self.log(format!("Connected peers: {:?}", peer_ids));
                    println!("Connected peers: {:?}", peer_ids);

                    let mut failed_agents = vec![];
                    let mut topic_switch = None;

                    // clone peer_info immutably then
                    // iterate and check which peers have stopped sending heartbeats
                    for (peer_id, peer_info) in self.peer_manager.peer_info.clone().iter() {

                        // let last_hb = peer_info.peer_heartbeat_data.last_heartbeat;
                        let duration = peer_info.peer_heartbeat_data.duration_since_last_heartbeat();
                        let node_name = get_node_name(&peer_id).magenta();
                        println!("{}\tlast seen {:.2?} seconds ago", node_name, duration);

                        if duration > max_time_before_respawn {
                            if let Some(AgentVessel { agent_name, next_vessel_peer_id, prev_vessel_peer_id }) = &peer_info.agent_vessel {

                                let respawn_pending = self.pending.respawns.contains(&(agent_name.clone(), *next_vessel_peer_id));

                                if respawn_pending {
                                    self.log(format!("Respawn pending: {} -> {}", agent_name, self.node_name));

                                } else {
                                    self.log(format!(
                                        "{} failed. Consensus: voting to reincarnate agent '{}'",
                                        node_name,
                                        agent_name
                                    ).magenta());

                                    // TODO: consensus mechanism to vote for reincarnation
                                    let failed_agent = agent_name;
                                    failed_agents.push(failed_agent);

                                    if self.peer_id == *next_vessel_peer_id {

                                        // Tell dispatch a Respawn(agent_name) request to eventloop
                                        let _ = self.network_event_sender
                                            .send(NetworkLoopEvent::Respawn(agent_name.to_owned(), prev_vessel_peer_id.clone())).await;

                                        self.log(format!("{} '{}' {} {}", "Respawning agent".blue(), agent_name.red(), "in new vessel:".blue(), self.node_name.yellow()));

                                        self.pending.respawns.insert((agent_name.to_owned(), *next_vessel_peer_id));
                                        // TODO: once respawn is pending,
                                        // 1) unsubscribe from Topic
                                        // 2) remove Vessel from PeerManagers
                                        // 3) broadcast peers to tell them to do the same


                                        topic_switch = Some(TopicSwitch {
                                            prev_topic: agent_name.to_string(), //
                                            next_topic: agent_name.to_string(), // +1 increment AgentName nonce
                                            prev_vessel_peer_id: prev_vessel_peer_id.clone()
                                        });

                                        // let _ = self.network_event_sender
                                        //     .send(NetworkLoopEvent::ReBroadcastKfrags(agent_name.to_owned())).await;
                                        // TODO: once broadcast happens, tell peers to update their PeerManager fields
                                        // - kfrags_peers
                                        // - peers_to_agent_frags
                                        // - peer_info

                                    }

                                    self.remove_peer(prev_vessel_peer_id);
                                    // self.peer_manager.remove_kfrags_peers_by_agent_name(agent_name);
                                }

                            } else {
                                println!("{} failed but wasn't hosting an agent.", node_name);
                            }
                        }
                    };

                    if let Some(topic_switch2) = topic_switch {
                        println!("Todo: broadcast topic switch");
                        // self.broadcast_topic_switch(topic_switch2).await;
                    }

                }
                swarm_event = self.swarm.select_next_some() => self.handle_swarm_event(swarm_event).await,
                heartbeat = self.internal_heartbeat_fail_receiver.recv() => match heartbeat {
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

    async fn handle_internal_heartbeat_failure(&mut self, heartbeat: String) {
        // Internal heartbeat failure is when this node fails to send heartbeats to external
        // nodes. It realizes it is no longer connected to the network.
        self.log(format!("{}", heartbeat));
        println!("\tTodo: initiating LLM runtime shutdown...");
        println!("\tTodo: attempt to broadcast agent_secrets reencryption fragments...");
        println!("\tTodo: Delete agent secrets, to prevent duplicate agents.");
        // Shutdown the LLM runtime (if in Vessel Mode), but
        // continue attempting to broadcast the agent_secrets reencryption fragments and ciphertexts.
        //
        // If the node never reconnects to the network, then 1up-network nodes will form consensus that
        // the Vessel is dead, and begin reincarnating the Agent from it's last public agent_secret ciphertexts
        // on the Kademlia network.
        //
        // In this case, the node should also delete it's Agent secrets to ensure that only
        // one agent is alive at a time.
    }

    fn remove_peer(&mut self, peer_id: &PeerId) {
        // Remove from PeerManager locally
        self.peer_manager.remove_kfrags_peer(peer_id);
        self.peer_manager.remove_peer_info(peer_id);

        // Remove UmbralPeerId of the Peer on Kademlia
        self.swarm
            .behaviour_mut()
            .kademlia
            .remove_record(&kad::RecordKey::new(&UmbralPeerId::from(peer_id).to_string()));
    }

    async fn broadcast_topic_switch(&mut self, topic_switch: TopicSwitch) {

        let match_topic = topic_switch.next_topic.clone();
        let gossip_topic = GossipTopic::TopicSwitch(topic_switch);
        let gossip_topic_bytes = serde_json::to_vec(&gossip_topic)
            .expect("serde err");


        match self.topics.get(&match_topic.to_string()) {
            Some(topic) => {
                let _ = self.swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), gossip_topic_bytes)
                    .map_err(|e| anyhow!(e.to_string()));
            }
            None => {
                self.log(format!("Topic '{}' not found in subscribed topics", match_topic));
                self.print_subscribed_topics();
            }
        }

        // pub struct PeerManager {
        //     // Tracks which Peers hold which AgentFragments
        //     // { agent_name: { frag_num: [PeerId] }}
        //     kfrags_peers: HashMap<AgentName, HashMap<FragmentNumber, HashSet<PeerId>>>,
        //     // Tracks which AgentFragments a specific Peer holds
        //     peers_to_agent_frags: HashMap<PeerId, HashSet<AgentFragment>>,
        //     // Tracks Vessel Nodes
        //     pub peer_info: HashMap<PeerId, PeerInfo>,
        // }

    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicSwitch {
    // unsubscribe from prev_topic
    pub prev_topic: String,
    // subscribe to next_topic
    pub next_topic: String,
    // remove kfrags_peers for old agent

    // remove peer from peer_info and peers_to_agent_frags
    pub prev_vessel_peer_id: PeerId
}