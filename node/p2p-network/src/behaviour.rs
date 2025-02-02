use color_eyre::Result;
use libp2p::{
    gossipsub::{self},
    kad,
    mdns,
    PeerId,
    request_response::{self},
    swarm::NetworkBehaviour
};
use serde::{Deserialize, Serialize};
use umbral_pre::KeyFrag;
use crate::types::{FragmentRequest, FragmentResponse, GossipTopic};
use crate::event_loop::heartbeat_behaviour;


/// Handles all p2p protocols
#[derive(NetworkBehaviour)]
pub struct Behaviour {

    // /// The Behaviour to manage connections to blocked peers.
    // blocked_peer: allow_block_list::Behaviour<allow_block_list::BlockedPeers>,

    pub request_response: request_response::cbor::Behaviour<FragmentRequest, FragmentResponse>,

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


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFragmentMessage {
    pub topic: GossipTopic,
    pub frag_num: usize,
    pub threshold: usize,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
    // TODO: split data into private data for the MPC node, vs public data for kademlia
    // private: kfrags, verify_pk, alice_pk, bob_pk -> store within the MPC node
    // public: capsules and ciphertexts -> store on Kademlia
    pub vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
    pub kfrag: KeyFrag,
    pub capsule: Option<umbral_pre::Capsule>,
    pub ciphertext: Option<Box<[u8]>>
}

