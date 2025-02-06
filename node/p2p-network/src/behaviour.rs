use color_eyre::Result;
use libp2p::{
    gossipsub,
    kad,
    mdns,
    request_response,
    swarm::NetworkBehaviour
};
use crate::types::{FragmentRequestEnum, FragmentResponseEnum};
use crate::event_loop::heartbeat_behaviour;


/// Handles all p2p protocols
#[derive(NetworkBehaviour)]
pub struct Behaviour {

    // /// The Behaviour to manage connections to blocked peers.
    // blocked_peer: allow_block_list::Behaviour<allow_block_list::BlockedPeers>,

    pub request_response: request_response::cbor::Behaviour<FragmentRequestEnum, FragmentResponseEnum>,

    /// Stores Umbra public keys for peers, and storing agent secret ciphertexts
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,

    /// Discovers peers via mDNS
    pub mdns: mdns::tokio::Behaviour,

    /// Handles regular heartbeats from peers
    pub heartbeat: heartbeat_behaviour::HeartbeatBehaviour,

    // /// The Behaviour to identify peers.
    // identify: identify::Behaviour,

    // /// Identifies and periodically requests `BlockHeight` from connected nodes
    // peer_report: peer_report::Behaviour,

    // /// Node discovery
    // discovery: discovery::Behaviour,

    /// Message propagation for threshold key generation and proxy re-encryption
    pub gossipsub: gossipsub::Behaviour
}
