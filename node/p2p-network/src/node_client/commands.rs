use std::collections::{HashMap, HashSet};
use color_eyre::{Result, eyre::anyhow};
use libp2p::{
    request_response::ResponseChannel,
    PeerId,
    Multiaddr
};
use tokio::sync::{mpsc, oneshot};
use crate::SendError;
use crate::types::{
    AgentNameWithNonce,
    FragmentNumber,
    FragmentResponseEnum,
    ReverieKeyfragMessage,
    NodeVesselWithStatus,
    ReverieId,
    ReverieMessage,
};
use super::container_manager::RestartReason;


pub enum NodeCommand {

    /// Gets the VesselStatus (which agent nodes are hosting),
    /// and Umbral PublicKey(s) of peers from Kademlia
    GetNodeVesselStatusesFromKademlia {
        sender: mpsc::Sender<NodeVesselWithStatus>,
    },

    /// Gets the ReverieId for an agent from Kademlia
    GetReverieIdFromAgentName {
        agent_name_nonce: AgentNameWithNonce,
        sender: oneshot::Sender<Option<ReverieId>>,
    },

    /// Gets the Reverie for an agent from Kademlia
    GetReverie {
        reverie_id: ReverieId,
        sender: oneshot::Sender<Result<ReverieMessage>>,
    },

    /// Gets Peers that hold the Kfrags for an agent.
    /// KfragProviders = kfrag holders
    /// KfragBroadcastPeers = peers subscribed to a Kfrag broadcast channel
    GetKfragProviders {
        reverie_id: ReverieId,
        sender: oneshot::Sender<HashSet<PeerId>>,
    },

    /// Sends Reverie Kfrags to specific peers
    SendReverieKeyfrag {
        keyfrag_provider: PeerId, // Key Fragment Provider
        reverie_keyfrag_msg: ReverieKeyfragMessage,
        agent_name_nonce: Option<AgentNameWithNonce>,
    },

    /// Sends a Reverie to a specific peer
    SendReverie {
        ciphertext_holder: PeerId, // Ciphertext Holder
        reverie_msg: ReverieMessage,
        agent_name_nonce: Option<AgentNameWithNonce>,
    },

    /// Request Capsule Fragments for threshold decryption
    RequestCapsuleFragment {
        reverie_id: ReverieId,
        kfrag_provider_peer_id: PeerId, // peer to request fragment from
        signature: umbral_pre::Signature,
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },

    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn std::error::Error + Send>>>,
    },

    SimulateNodeFailure {
        sender: oneshot::Sender<RestartReason>,
        reason: RestartReason,
    },

    GetNodeState {
        sender: oneshot::Sender<serde_json::Value>,
    },

    MarkPendingRespawnComplete {
        prev_reverie_id: ReverieId,
        prev_peer_id: PeerId,
        prev_agent_name_nonce: AgentNameWithNonce,
    },
}