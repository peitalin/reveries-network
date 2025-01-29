use std::fmt::Display;
use regex::Regex;
use color_eyre::Result;
use libp2p::{
    gossipsub::{self, IdentTopic, TopicHash},
    kad,
    mdns,
    PeerId,
    request_response::{self, ResponseChannel},
    swarm::NetworkBehaviour
};
use serde::{Deserialize, Serialize};
use umbral_pre::KeyFrag;
use crate::types::UmbralPeerId;
use crate::event_loop::heartbeat_behaviour;
use crate::{AgentName, FragmentNumber, AGENT_DELIMITER};

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


#[derive(Debug)]
pub enum FileEvent {
    InboundRequest {
        request: String,
        frag_num: Option<usize>,
        channel: ResponseChannel<FragmentResponse>
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRequest(pub String, pub Option<usize>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentResponse(pub Vec<u8>);


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFragmentMessage {
    pub topic: KfragsTopic,
    pub frag_num: usize,
    pub threshold: usize,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
    // TODO: split data into private data for the MPC node, vs public data for kademlia
    // private: kfrags, verify_pk, alice_pk, bob_pk -> store within the MPC node
    // public: capsules and ciphertexts -> store on Kademlia
    pub vessel_peer_id: PeerId,
    pub kfrag: KeyFrag,
    pub capsule: Option<umbral_pre::Capsule>,
    pub ciphertext: Option<Box<[u8]>>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleFragmentIndexed {
    pub frag_num: usize,
    pub threshold: usize,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,

    pub vessel_peer_id: PeerId,
    pub cfrag: umbral_pre::CapsuleFrag,
    pub capsule: Option<umbral_pre::Capsule>,
    pub ciphertext: Option<Box<[u8]>>,
}

impl Display for CapsuleFragmentIndexed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,
            "CapsuleFragmentIndexed(frag_num: {}, alice_pk: {}, bob_pk: {}, verifying_pk, cfrag, capsule, ciphertext)",
            self.frag_num,
            self.alice_pk,
            self.bob_pk,
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct UmbralPublicKeyResponse {
    pub umbral_peer_id: UmbralPeerId,
    pub umbral_public_key: umbral_pre::PublicKey,
}

impl Display for UmbralPublicKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UmbralPublicKeyResponse({}, {})", self.umbral_peer_id, self.umbral_public_key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KfragsTopic {
    BroadcastKfrag(AgentName, FragmentNumber),
    RequestCfrags(AgentName),
    Unknown(String),
}

impl Display for KfragsTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::BroadcastKfrag(agent_name, frag_num) => write!(f, "kfrag{}{}{}", frag_num, d, agent_name),
            Self::RequestCfrags(agent_name) => write!(f, "request{}{}", d, agent_name),
            Self::Unknown(s) => write!(f, "{}", s),
        }
    }
}

impl Into<String> for KfragsTopic {
    fn into(self) -> String {
        let d = AGENT_DELIMITER;
        match self {
            Self::BroadcastKfrag(agent_name, frag_num) => format!("kfrag{}{}{}", frag_num, d, agent_name),
            Self::RequestCfrags(agent_name) => format!("request{}{}", d, agent_name),
            Self::Unknown(s) => format!("{}", s),
        }
    }
}

// Temporary way to issue chat commands to the node and test features in development
// which will later be replaced with automated heartbeats, protocols, etc;
pub(crate) fn split_topic_by_delimiter(topic_str: &str) -> (&str, &str, Option<(usize, usize)>) {
    match topic_str {
        "chat" => ("chat", "", None),
        "unknown" => ("unknown", "", None),
        _ => {
            let mut tsplit = topic_str.split(AGENT_DELIMITER);
            let topic = tsplit.next().unwrap_or("unknown_topic");
            let agent_name = tsplit.next().unwrap_or("");
            let nshare_threshold: Option<(usize, usize)> = match tsplit.next() {
                None => None,
                Some(nt) => {
                    let re = Regex::new(r"\(([0-9]*),([0-9]*)\)").unwrap();
                    if let Some((_, [n, t])) = re.captures(nt).map(|c| c.extract()) {
                        let nshares = n.parse().unwrap();
                        let threshold = t.parse().unwrap();
                        Some((nshares, threshold))
                    } else {
                        None
                    }
                }
            };
            (topic, agent_name, nshare_threshold)
        }
    }
}

impl From<String> for KfragsTopic {
    fn from(s: String) -> Self {

        let (topic, agent_name, _) = split_topic_by_delimiter(&s);
        let agent_name = agent_name.to_string();

        if topic.starts_with("kfrag") {
            let frag_num = match topic.chars().last().expect("topic.chars.last err").to_digit(10) {
                None => panic!("kfrag topic should have a frag number: {}", topic),
                Some(n) => n
            };
            KfragsTopic::BroadcastKfrag(agent_name, frag_num)
        } else if topic.starts_with("request") {
            KfragsTopic::RequestCfrags(agent_name)
        } else {
            KfragsTopic::Unknown(topic.to_string())
        }
    }
}

impl Into<IdentTopic> for KfragsTopic {
    fn into(self) -> IdentTopic {
        gossipsub::IdentTopic::new(self.to_string())
    }
}

impl Into<TopicHash> for KfragsTopic {
    fn into(self) -> TopicHash {
        <KfragsTopic as Into<IdentTopic>>::into(self).into()
    }
}

impl From<TopicHash> for KfragsTopic {
    fn from(i: TopicHash) -> KfragsTopic {
        let s = i.to_string();
        s.into()
    }
}
