pub(crate) mod heartbeat_behaviour;

use color_eyre::Result;
use libp2p::{
    gossipsub,
    kad,
    request_response,
    swarm::NetworkBehaviour
};
use crate::types::{FragmentRequestEnum, FragmentResponseEnum};


/// Handles all p2p protocols
#[derive(NetworkBehaviour)]
pub struct Behaviour {

    // /// The Behaviour to manage connections to blocked peers.
    // blocked_peer: allow_block_list::Behaviour<allow_block_list::BlockedPeers>,

    pub request_response: request_response::cbor::Behaviour<FragmentRequestEnum, FragmentResponseEnum>,

    /// Stores Umbra public keys for peers, and storing agent secret ciphertexts
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,

    /// Handles regular heartbeats from peers
    pub heartbeat: heartbeat_behaviour::HeartbeatBehaviour,

    /// The Behaviour to identify peers.
    pub identify: libp2p_identify::Behaviour,

    /// Deprecated in favor of request-response protocol
    pub gossipsub: gossipsub::Behaviour
}
