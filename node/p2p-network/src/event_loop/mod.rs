mod chat;
mod swarm_handlers;
mod command_handlers;
mod gossipsub_handlers;
pub(crate) mod heartbeat_behaviour;
pub(crate) mod peer_manager;

use std::collections::{HashMap, HashSet};
use color_eyre::Result;
use colored::Colorize;
use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use libp2p::{
    gossipsub::IdentTopic,
    kad,
    request_response,
    swarm::Swarm,
    PeerId
};
use runtime::reencrypt::UmbralKey;
use crate::SendError;
use crate::{node_client::NodeCommand, get_node_name, short_peer_id};
use crate::types::{
    AgentName,
    ChatMessage,
    TopicSwitch,
    GossipTopic,
    UmbralPeerId,
    NetworkLoopEvent,
    UmbralPublicKeyResponse
};
use crate::create_network::NODE_SEED_NUM;
use crate::behaviour::Behaviour;
use crate::event_loop::peer_manager::AgentVessel;
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
use runtime::llm::{AgentSecretsJson, test_claude_query};

pub struct EventLoop {
    seed: usize,
    swarm: Swarm<Behaviour>,
    peer_id: PeerId, // My node's PeerInfo
    node_name: String,
    command_receiver: mpsc::Receiver<NodeCommand>,
    chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
    network_event_sender: mpsc::Sender<NetworkLoopEvent>,

    // Umbral fragments
    umbral_key: UmbralKey,

    internal_heartbeat_fail_receiver: tokio::sync::mpsc::Receiver<String>,

    interval: time::Interval,

    // tracks peer heartbeats status
    peer_manager: PeerManager,

    // pending P2p network requests
    pending: PendingRequests,

    topics: HashMap<String, IdentTopic>,
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
    respawns: HashSet<(AgentName, PeerId)>,
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
        seed: usize,
        swarm: Swarm<Behaviour>,
        peer_id: PeerId,
        node_name: String,
        command_receiver: mpsc::Receiver<NodeCommand>,
        chat_cmd_receiver: mpsc::Receiver<ChatMessage>,
        network_event_sender: mpsc::Sender<NetworkLoopEvent>,
        umbral_key: UmbralKey,
        internal_heartbeat_fail_receiver: tokio::sync::mpsc::Receiver<String>,
    ) -> Self {

        Self {
            seed,
            swarm,
            peer_id,
            node_name,
            command_receiver,
            chat_cmd_receiver,
            network_event_sender,
            umbral_key: umbral_key,
            internal_heartbeat_fail_receiver,
            interval: tokio::time::interval(Duration::from_secs(1)),
            peer_manager: PeerManager::new(),
            pending: PendingRequests::new(),
            topics: HashMap::new(),
        }
    }

    fn log<S: std::fmt::Display>(&self, message: S) {
        println!("{} {}{} {}",
            "NetworkEvent".green(), self.node_name.yellow(), ">".blue(),
            message
        );
    }

    pub async fn listen_for_network_events(mut self) {

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
                    let mut topic_switch: Option<TopicSwitch> = None;

                    // iterate and check which peers have stopped sending heartbeats
                    for (peer_id, peer_info) in self.peer_manager.peer_info.clone().iter() {

                        // let last_hb = peer_info.peer_heartbeat_data.last_heartbeat;
                        let duration = peer_info.peer_heartbeat_data.duration_since_last_heartbeat();
                        let node_name = get_node_name(&peer_id).magenta();
                        println!("{}\tlast seen {:.2?} seconds ago", node_name, duration);

                        if duration > max_time_before_respawn {
                            if let Some(AgentVessel {
                                agent_name,
                                agent_nonce,
                                total_frags,
                                next_vessel_peer_id,
                                prev_vessel_peer_id
                            }) = &peer_info.agent_vessel {

                                let agent_name_nonce_key = format!("{agent_name}-{agent_nonce}");
                                let next_nonce = agent_nonce + 1;
                                let respawn_pending = self.pending.respawns.contains(&(agent_name_nonce_key.clone(), *next_vessel_peer_id));

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

                                    // if this node is the next vessel for the agent
                                    if self.peer_id == *next_vessel_peer_id {

                                        // subscribe to all new agent_nonce channels to broadcast
                                        self.subscribe_topics(vec![
                                            GossipTopic::BroadcastKfrag(agent_name.clone(), next_nonce, *total_frags, 0).to_string(),
                                            GossipTopic::BroadcastKfrag(agent_name.clone(), next_nonce, *total_frags, 1).to_string(),
                                            GossipTopic::BroadcastKfrag(agent_name.clone(), next_nonce, *total_frags, 2).to_string(),
                                            GossipTopic::BroadcastKfrag(agent_name.clone(), next_nonce, *total_frags, 3).to_string(),
                                        ]).await;

                                        // Dispatch a Respawn(agent_name) event to EventLoop
                                        let _ = self.network_event_sender
                                            .send(NetworkLoopEvent::RespawnRequired {
                                                agent_name: agent_name.to_owned(),
                                                agent_nonce: agent_nonce.clone(),
                                                total_frags: *total_frags,
                                                prev_peer_id: prev_vessel_peer_id.clone()
                                            }).await;

                                        self.log(format!("{} '{}' {} {}", "Respawning agent".blue(),
                                            agent_name.green(),
                                            "in new vessel:".blue(),
                                            self.node_name.yellow()
                                        ));

                                        // mark as respawning...
                                        self.pending.respawns.insert((agent_name_nonce_key, *next_vessel_peer_id));

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
                                        //     .send(NetworkLoopEvent::ReBroadcastKfrags(agent_name.to_owned())).await;
                                        // TODO: once broadcast happens, tell peers to update their PeerManager fields
                                        // - kfrags_peers
                                        // - peers_to_agent_frags
                                        // - peer_info

                                    } else {

                                        // If no is not the next vessel:
                                        // Subscribe to next agent_nonce channel for when it
                                        // is reincarnated and broadcasting cfrags
                                        // let frag_num: usize = NODE_SEED_NUM.with(|n: &std::cell::RefCell<usize>| {
                                        //     *n.borrow() % total_frags
                                        // });
                                        let frag_num = self.seed % total_frags;
                                        self.log(format!("\n\nNEXT FRAG_NUM: {}\n", frag_num));

                                        self.subscribe_topics(vec![
                                            GossipTopic::BroadcastKfrag(agent_name.clone(), next_nonce, *total_frags, frag_num).to_string(),
                                        ]).await;
                                    }

                                    // remove peer kfrags, it will disconnect automatically after a while
                                    self.remove_peer(peer_id);
                                    // self.peer_manager.remove_kfrags_peers_by_agent_name(agent_name, *agent_nonce);
                                }

                            } else {
                                println!("{} failed but wasn't hosting an agent.", node_name);
                                // remove peer kfrags, it will disconnect automatically after a while
                                // self.remove_peer(peer_id);
                            }
                        }
                    };

                    if let Some(topic_switch2) = topic_switch {
                        self.log(format!("Broadcasting topic switch"));
                        self.broadcast_topic_switch(topic_switch2).await;
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

}
