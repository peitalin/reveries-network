use std::collections::{HashMap, HashSet};
use std::error::Error;
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
    KeyFragmentMessage,
    TopicSwitch,
    NodeVesselWithStatus,
    ReverieId,
};
use super::container_manager::RestartReason;


pub enum NodeCommand {

    /// Gets the VesselStatus (which agent nodes are hosting),
    /// and Umbral PublicKey(s) of peers from Kademlia
    GetNodeVesselStatusesFromKademlia {
        sender: mpsc::Sender<NodeVesselWithStatus>,
    },

    /// Gets the ReverieId for an agent from Kademlia
    GetAgentReverieId {
        agent_name_nonce: AgentNameWithNonce,
        sender: oneshot::Sender<Option<ReverieId>>,
    },

    /// Gets Peers that hold the Kfrags for an agent.
    /// KfragProviders = kfrag holders
    /// KfragBroadcastPeers = peers subscribed to a Kfrag broadcast channel
    GetKfragProviders {
        reverie_id: ReverieId,
        // sender: oneshot::Sender<HashSet<PeerId>>,
        // returns: {frag_num: [peer_id]}
        sender: oneshot::Sender<HashMap<usize, HashSet<PeerId>>>,
    },

    /// Saves the provider of the kfrag for retrieval later
    SaveKfragProvider {
        reverie_id: ReverieId,
        // agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId, // peer who holds the kfrag
        channel: ResponseChannel<FragmentResponseEnum>,
    },

    /// Broadcasts to peers to listen to a new kfrag broadcast channel
    /// when spawning a new agent, or reincarnating a agent with a new nonce.
    /// Protocol usually broadcasts a topic switch before broadcasting kfrags.
    BroadcastSwitchTopic(
        TopicSwitch,
        oneshot::Sender<usize>,
    ),

    /// Broadcasts Kfrags to peers (multicasts to specific fragment channels)
    /// Kfrags are verified then encrypted
    BroadcastKfrags(KeyFragmentMessage),

    /// Request Capsule Fragments for threshold decryption
    RequestCapsuleFragment {
        reverie_id: ReverieId,
        // agent_name_nonce: AgentNameWithNonce,
        frag_num: FragmentNumber,
        peer: PeerId, // peer to request fragment from
        sender: oneshot::Sender<Result<Vec<u8>, SendError>>,
    },

    RespondCapsuleFragment {
        reverie_id: ReverieId,
        // agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId, // peer who sends cfrag back
        channel: ResponseChannel<FragmentResponseEnum>,
    },

    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    UnsubscribeTopics {
        topics: Vec<String>,
        sender: oneshot::Sender<Vec<String>>,
    },
    SimulateNodeFailure {
        sender: oneshot::Sender<RestartReason>,
        reason: RestartReason,
    },

    GetNodeState {
        sender: oneshot::Sender<serde_json::Value>,
    },

    /// Gets all addresses this node is listening on
    GetListeningAddresses {
        sender: oneshot::Sender<Vec<Multiaddr>>,
    },

    /// Gets all peers this node is connected to
    GetConnectedPeers {
        sender: oneshot::Sender<Vec<PeerId>>,
    },
}