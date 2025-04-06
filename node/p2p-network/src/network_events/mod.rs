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
    identity,
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
    FragmentNumber,
    GossipTopic,
    NetworkEvent,
    RespawnId,
    VesselPeerId,
    AgentVesselInfo,
    NodeVesselWithStatus,
    VesselStatus,
    SignedVesselStatus,
    ReverieId,
    AgentReverieId,
};
use crate::node_client::container_manager::{ContainerManager, RestartReason};
use crate::create_network::NODE_SEED_NUM;
use crate::behaviour::Behaviour;
use runtime::reencrypt::UmbralKey;
use peer_manager::PeerManager;
use tokio::time;
use time::Duration;
// use runtime::llm::test_claude_query;


#[derive(Clone)]
pub struct NodeIdentity<'a> {
    pub node_name: &'a str,
    pub peer_id: PeerId,
    pub id_keys: identity::Keypair,
    seed: usize,
    umbral_key: UmbralKey,
}

impl<'a> NodeIdentity<'a> {
    pub fn new(
        node_name: &'a str,
        peer_id: PeerId,
        id_keys: identity::Keypair,
        seed: usize,
        umbral_key: UmbralKey,
    ) -> Self {
        Self {
            node_name,
            peer_id,
            id_keys,
            seed,
            umbral_key,
        }
    }
}

pub struct NetworkEvents<'a> {
    // Node identity
    node_id: NodeIdentity<'a>,
    // IPFS Swarm
    swarm: Swarm<Behaviour>,
    // Command receiver
    command_receiver: mpsc::Receiver<NodeCommand>,
    // Network event sender
    network_event_sender: mpsc::Sender<NetworkEvent>,
    // tracks own heartbeat status
    internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,
    // tracks peer heartbeats status
    peer_heartbeat_checker: time::Interval,
    // Peer Manager State
    peer_manager: PeerManager,
    // pending P2p network requests
    pending: PendingRequests,
    // GossipSub topics
    topics: HashMap<String, IdentTopic>,
    // Container Manager
    container_manager: Arc<RwLock<ContainerManager>>
}

struct PendingRequests {
    get_providers: HashMap<
        kad::QueryId,
        oneshot::Sender<HashSet<PeerId>>
    >,
    get_node_vessels: HashMap<
        VesselPeerId,
        mpsc::Sender<NodeVesselWithStatus>
    >,
    get_agent_reverie_id: HashMap<
        AgentReverieId,
        oneshot::Sender<Option<ReverieId>>
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
            get_node_vessels: Default::default(),
            get_agent_reverie_id: Default::default(),
            request_fragments: Default::default(),
            respawns: Default::default(),
        }
    }
}

impl<'a> NetworkEvents<'a> {
    pub fn new(
        swarm: Swarm<Behaviour>,
        node_id: NodeIdentity<'a>,
        command_receiver: mpsc::Receiver<NodeCommand>,
        network_event_sender: mpsc::Sender<NetworkEvent>,
        internal_heartbeat_fail_receiver: mpsc::Receiver<HeartbeatConfig>,
        container_manager: Arc<RwLock<ContainerManager>>
    ) -> Self {
        let node_name = node_id.node_name.to_string();
        let peer_id = node_id.peer_id.clone();
        Self {
            swarm,
            node_id,
            command_receiver,
            network_event_sender,
            internal_heartbeat_fail_receiver,
            peer_heartbeat_checker: tokio::time::interval(Duration::from_secs(1)),
            peer_manager: PeerManager::new(node_name, peer_id),
            pending: PendingRequests::new(),
            topics: HashMap::new(),
            container_manager,
        }
    }

    fn nname(&self) -> String {
        format!("{}{}", self.node_id.node_name.yellow(), ">".blue())
    }

    pub async fn listen_for_network_events(mut self) {
        // Start listening on specified addresses
        for addr in self.swarm.listeners() {
            info!("{} {}", self.nname(), format!("Listening on {}", addr));
        }
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
                    self.handle_swarm_event(swarm_event).await.expect("swarm handler error");
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
            }
        }
    }

    async fn handle_peer_heartbeat_failure(&mut self) -> Result<()> {

        let max_time_before_respawn = self.swarm.behaviour()
            .heartbeat
            .config
            .max_time_before_rotation();

        let connected_peers = self.swarm.connected_peers()
            .map(|p| format!("{}", get_node_name(p)))
            .collect::<Vec<String>>();

        info!("{} connected peers: {:?}", self.nname(), connected_peers);

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
                    // will automatically remove_peer() after timeout
                    // self.remove_peer(&peer_id);
                }

                if let Some(AgentVesselInfo {
                    agent_name_nonce: prev_agent,
                    total_frags,
                    next_vessel_peer_id,
                    current_vessel_peer_id
                }) = &peer_info.agent_vessel {

                    let next_agent = AgentNameWithNonce(prev_agent.0.clone(), prev_agent.1 + 1);
                    info!("{}", format!("{} failed. Consensus: voting to reincarnate agent '{}'",
                        node_name,
                        prev_agent
                    ).magenta());

                    // TODO: consensus mechanism to vote for reincarnation
                    failed_agents_placeholder.push(prev_agent);

                    // mark as respawning...
                    let respawn_id = RespawnId::new(&prev_agent, &current_vessel_peer_id);
                    self.pending.respawns.insert(respawn_id.clone());

                    ////////////////////////////////////////////////
                    // If this node is the next vessel for the agent
                    if self.node_id.peer_id == *next_vessel_peer_id {

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
                        self.network_event_sender
                            .send(NetworkEvent::RespawnRequest(
                                AgentVesselInfo {
                                    agent_name_nonce: prev_agent.clone(),
                                    total_frags: *total_frags,
                                    next_vessel_peer_id: *next_vessel_peer_id,
                                    current_vessel_peer_id: current_vessel_peer_id.clone()
                                }
                            )).await?;

                        info!("{} '{}' {} {}", "Respawning agent".blue(),
                            prev_agent.to_string().green(),
                            "in new vessel:".blue(),
                            self.node_id.node_name.yellow()
                        );

                        // 1) remove Vessel from PeerManagers and Swarm
                        self.remove_peer(&peer_id);

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
                            info!("Respawn pending: {} -> {}", next_agent, self.node_id.node_name);

                        } else {

                            let frag_num = self.node_id.seed % total_frags;
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
        self.peer_manager.remove_kfrag_provider(peer_id);
        self.peer_manager.remove_peer_info(peer_id);
        // Remove peer from GossipSub
        self.swarm.behaviour_mut()
            .gossipsub
            .remove_explicit_peer(&peer_id);
        // Remove VesselPeerId of the Peer on Kademlia
        self.swarm.behaviour_mut()
            .kademlia
            .remove_record(&kad::RecordKey::new(&VesselPeerId::from(peer_id).to_string()));
    }

    pub(crate) async fn simulate_heartbeat_failure(&mut self) {
        self.swarm.behaviour_mut()
            .heartbeat
            .trigger_heartbeat_failure().await;
    }

    fn put_signed_vessel_status(&mut self, status: NodeVesselWithStatus) -> Result<()> {
        let signed_status = SignedVesselStatus::new(status.clone(), &self.node_id.id_keys)?;

        self.swarm.behaviour_mut().kademlia.put_record(
            kad::Record {
                key: kad::RecordKey::new(&VesselPeerId::from(status.peer_id).to_string()),
                value: serde_json::to_vec(&signed_status)?,
                publisher: Some(status.peer_id),
                expires: None,
            },
            kad::Quorum::One
        )?;

        Ok(())
    }

}
