mod swarm_handlers;
mod command_handlers;
mod gossipsub_handlers;
mod kademlia_handlers;
mod request_response_handlers;
mod reincarnation;
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
    ReverieIdToAgentName,
    ReverieIdToPeerId,
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
    get_reverie_agent_name: HashMap<
        ReverieIdToAgentName,
        oneshot::Sender<Option<ReverieId>>
    >,
    get_reverie_peer_id: HashMap<
        ReverieIdToPeerId,
        oneshot::Sender<Option<PeerId>>
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
            get_reverie_agent_name: Default::default(),
            get_reverie_peer_id: Default::default(),
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

    pub(crate) fn remove_peer(&mut self, peer_id: &PeerId) {
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

    fn put_signed_vessel_status_kademlia(&mut self, status: NodeVesselWithStatus) -> Result<()> {
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

    fn put_reverie_holder_kademlia(&mut self, reverie_id: ReverieId, reverie_holder_peer_id: PeerId) -> Result<()> {
        let kad_key = ReverieIdToPeerId::from(reverie_id.clone());
        self.swarm.behaviour_mut().kademlia.put_record(
            kad::Record {
                key: kad::RecordKey::new(&kad_key.to_string()),
                value: reverie_holder_peer_id.to_bytes(),
                publisher: Some(self.node_id.peer_id),
                expires: None,
            },
            kad::Quorum::One
        )?;
        Ok(())
    }

}
