use std::{
    collections::{HashMap, HashSet}, fmt::Display, str::FromStr
};
use regex::Regex;
use anyhow::Result;
use libp2p::{
    gossipsub::{self, IdentTopic, TopicHash},
    kad,
    mdns,
    request_response::{self, ResponseChannel},
    swarm::NetworkBehaviour, PeerId
};
use serde::{Deserialize, Serialize};
use umbral_pre::KeyFrag;
use crate::event_loop::heartbeat;
use crate::{AgentName, FragmentNumber, AGENT_DELIMITER};


/// Handles all p2p protocols
#[derive(NetworkBehaviour)]
pub struct Behaviour {

    // /// The Behaviour to manage connections to blocked peers.
    // blocked_peer: allow_block_list::Behaviour<allow_block_list::BlockedPeers>,

    pub request_response: request_response::cbor::Behaviour<FileRequest, FileResponse>,

    /// Stores Umbra public keys for peers, and storing agent secret ciphertexts
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,

    /// Discovers peers via mDNS
    pub mdns: mdns::tokio::Behaviour,

    /// Handles regular heartbeats from peers
    pub heartbeat: heartbeat::Behaviour,

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
        frag_num: Option<u32>,
        channel: ResponseChannel<FileResponse>
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRequest(pub String, pub Option<u32>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileResponse(pub Vec<u8>);

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub topic: ChatTopic,
    pub message: String
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatTopic {
    Chat,
    Unknown,
}

impl Display for ChatTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Unknown => write!(f, "unknown topic"),
        }
    }

}

impl From<String> for ChatTopic {
    fn from(s: String) -> Self {
        match s.as_str() {
            "chat" => ChatTopic::Chat,
            _ => Self::Unknown
        }
    }
}

impl From<TopicHash> for ChatTopic {
    fn from(i: TopicHash) -> ChatTopic {
        let s = i.to_string();
        s.into()
    }
}

impl Into<String> for ChatTopic {
    fn into(self) -> String {
        match self {
            Self::Chat => format!("chat"),
            Self::Unknown => format!("unknown topic"),
        }
    }
}

impl Into<IdentTopic> for ChatTopic {
    fn into(self) -> IdentTopic {
        gossipsub::IdentTopic::new(self.to_string())
    }
}

impl Into<TopicHash> for ChatTopic {
    fn into(self) -> TopicHash {
        <ChatTopic as Into<IdentTopic>>::into(self).into()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KfragsBroadcastMessage {
    pub topic: KfragsTopic,
    pub frag_num: usize,
    pub threshold: usize,
    pub kfrag: KeyFrag,
    pub verifying_pk: umbral_pre::PublicKey,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub capsule: Option<umbral_pre::Capsule>,
    pub ciphertext: Option<Box<[u8]>>
}

impl Into<KeyFragmentIndexed> for KfragsBroadcastMessage {
    fn into(self) -> KeyFragmentIndexed {
        KeyFragmentIndexed {
            frag_num: self.frag_num,
            threshold: self.threshold,
            kfrag: self.kfrag,
            verifying_pk: self.verifying_pk,
            alice_pk: self.alice_pk,
            bob_pk: self.bob_pk,
            capsule: self.capsule,
            ciphertext: self.ciphertext
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFragmentIndexed {
    pub frag_num: usize,
    pub threshold: usize,
    pub kfrag: KeyFrag,
    pub verifying_pk: umbral_pre::PublicKey,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
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


#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct UmbralPeerId(pub String);

const UMBRAL_KEY_PREFIX: &'static str = "umbral_pubkey_";

impl From<String> for UmbralPeerId {
    fn from(s: String) -> Self {
        if s.starts_with(UMBRAL_KEY_PREFIX) {
            let ssplit = s.split(UMBRAL_KEY_PREFIX).collect::<Vec<&str>>();
            UmbralPeerId(ssplit[1].to_string())
        } else {
            panic!("Does not begin with {}: {}", UMBRAL_KEY_PREFIX, s)
        }
    }
}

impl From<&str> for UmbralPeerId {
    fn from<'a>(s: &'a str) -> Self {
        if s.starts_with(UMBRAL_KEY_PREFIX) {
            let ssplit = s.split(UMBRAL_KEY_PREFIX).collect::<Vec<&str>>();
            UmbralPeerId(ssplit[1].to_string())
        } else {
            panic!("Does not begin with {}: {}", UMBRAL_KEY_PREFIX, s)
        }
    }
}

impl Into<String> for UmbralPeerId {
    fn into(self) -> String {
        format!("{}{}", UMBRAL_KEY_PREFIX, self.0)
    }
}

impl Display for UmbralPeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", UMBRAL_KEY_PREFIX, self.0)
    }
}

impl From<PeerId> for UmbralPeerId {
    fn from(p: PeerId) -> Self {
        UmbralPeerId(p.to_string())
    }
}

impl Into<PeerId> for UmbralPeerId {
    fn into(self) -> PeerId {
        // slice off the UMBRAL_KEY_PREFIX first
        let umbral_peer_id_str = self.to_string();
        let peer_id_str = &umbral_peer_id_str.as_str()[UMBRAL_KEY_PREFIX.len()..];
        PeerId::from_str(peer_id_str)
            .expect("Error unwrapping UmbralPeerId to PeerId")
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct UmbralPublicKeyResponse {
    pub peer_id: UmbralPeerId,
    pub umbral_public_key: umbral_pre::PublicKey,
}

impl Display for UmbralPublicKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UmbralPublicKeyResponse({}, {})", self.peer_id, self.umbral_public_key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KfragsTopic {
    Kfrag(AgentName, FragmentNumber),
    RequestCfrags(String),
    Unknown(String),
}

impl Display for KfragsTopic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let d = AGENT_DELIMITER;
        match self {
            Self::Kfrag(agent_name, frag_num) => write!(f, "kfrag{}{}{}", frag_num, d, agent_name),
            Self::RequestCfrags(agent_name) => write!(f, "request{}{}", d, agent_name),
            Self::Unknown(s) => write!(f, "{}", s),
        }
    }
}

impl Into<Vec<u8>> for KfragsTopic {
    fn into(self) -> Vec<u8> {
        let topic_str = self.to_string();
        topic_str.as_bytes().to_vec()
    }
}

impl Into<String> for KfragsTopic {
    fn into(self) -> String {
        let d = AGENT_DELIMITER;
        match self {
            Self::Kfrag(agent_name, frag_num) => format!("kfrag{}{}{}", frag_num, d, agent_name),
            Self::RequestCfrags(agent_name) => format!("request{}{}", d, agent_name),
            Self::Unknown(s) => format!("{}", s),
        }
    }
}

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
            KfragsTopic::Kfrag(agent_name, frag_num)
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
