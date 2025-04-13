
use color_eyre::{eyre::anyhow, Result};
use serde::{Deserialize, Serialize};
use umbral_pre::Capsule;
use libp2p::{PeerId, kad};

use crate::utils::{
    reverie_id,
    REVERIE_ID_PREFIX,
};
use crate::types::{
    ReverieNameWithNonce,
    VesselPeerId,
    VESSEL_KAD_KEY_PREFIX,
};


pub type ReverieId = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverieKeyfragMessage {
    pub reverie_keyfrag: ReverieKeyfrag,
    pub source_peer_id: PeerId,
    pub target_peer_id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverieMessage {
    pub reverie: Reverie,
    pub source_peer_id: PeerId,
    pub target_peer_id: PeerId,
}


/// An encrypted memory module, used by an agent
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Reverie {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub description: String,
    pub threshold: usize,
    pub total_frags: usize,
    pub umbral_capsule: Vec<u8>,
    pub umbral_ciphertext: Box<[u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReverieKeyfrag {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub frag_num: usize,
    pub threshold: usize,
    pub total_frags: usize,
    pub umbral_keyfrag: Vec<u8>,
    pub umbral_capsule: Vec<u8>,
    pub source_pubkey: umbral_pre::PublicKey, // source_pubkey
    pub target_pubkey: umbral_pre::PublicKey,   // target_pubkey
    pub source_verifying_pubkey: umbral_pre::PublicKey, // source verifying key
    pub target_verifying_pubkey: umbral_pre::PublicKey,   // target verifying key
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReverieCapsulefrag {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub frag_num: usize,
    pub threshold: usize,
    pub umbral_capsule_frag: Vec<u8>,
    pub source_pubkey: umbral_pre::PublicKey, // source_pubkey
    pub target_pubkey: umbral_pre::PublicKey,   // target_pubkey
    pub source_verifying_pubkey: umbral_pre::PublicKey, // source verifying key
    pub target_verifying_pubkey: umbral_pre::PublicKey,   // target verifying key
    pub kfrag_provider_peer_id: PeerId,
}

impl ReverieCapsulefrag {
    pub fn encode_capsule_frag(&self) -> Result<umbral_pre::CapsuleFrag> {
        serde_json::from_slice(&self.umbral_capsule_frag)
            .map_err(|e| anyhow!("Error deserializing CapsuleFrag: {}", e))
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReverieType {
    SovereignAgent(ReverieNameWithNonce),
    Agent(ReverieNameWithNonce),
    Memory,
    Retrieval,
    Tools,
}

impl ReverieType {
    pub fn to_string(&self) -> String {
        self.clone().into()
    }
}

impl Into<String> for ReverieType {
    fn into(self) -> String {
        match self {
            ReverieType::SovereignAgent(agent_name_nonce) => agent_name_nonce.into(),
            ReverieType::Agent(agent_name_nonce) => agent_name_nonce.into(),
            ReverieType::Memory => "Memory".to_string(),
            ReverieType::Retrieval => "Retrieval".to_string(),
            ReverieType::Tools => "Tools".to_string(),
        }
    }
}

impl Reverie {
    pub fn new(
        description: String,
        reverie_type: ReverieType,
        threshold: usize,
        total_frags: usize,
        capsule: umbral_pre::Capsule,
        ciphertext: Box<[u8]>
    ) -> Self {
        Self {
            id: reverie_id(),
            reverie_type: reverie_type,
            description: description,
            threshold: threshold,
            total_frags: total_frags,
            umbral_capsule: serde_json::to_vec(&capsule).expect("Failed to serialize capsule"),
            umbral_ciphertext: ciphertext
        }
    }

    pub fn encode_capsule(&self) -> Result<umbral_pre::Capsule> {
        serde_json::from_slice(&self.umbral_capsule)
            .map_err(|e| anyhow!("Error deserializing Capsule: {}", e))
    }
}



#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToAgentName(pub String);

pub const REVERIE_ID_TO_NAME_KADKEY_PREFIX: &'static str = "reverie_to_name";


impl Into<String> for ReverieIdToAgentName {
    fn into(self) -> String {
        format!("{}{}", REVERIE_ID_TO_NAME_KADKEY_PREFIX, self.0)
    }
}

impl From<ReverieNameWithNonce> for ReverieIdToAgentName {
    fn from(agent_name_nonce: ReverieNameWithNonce) -> Self {
        ReverieIdToAgentName(agent_name_nonce.to_string())
    }
}

impl ReverieIdToAgentName {
    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(REVERIE_ID_TO_NAME_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_NAME_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToAgentName(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToAgentName: {}. Must begin with {}", s, REVERIE_ID_TO_NAME_KADKEY_PREFIX))
        }
    }

    pub fn to_kad_key(&self) -> kad::RecordKey {
        kad::RecordKey::new(&self.to_string())
    }

    pub fn to_string(&self) -> String {
        self.clone().into()
    }

    pub fn is_kademlia_key<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(REVERIE_ID_TO_NAME_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_NAME_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToAgentName(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToAgentName kademlia key: {}. Must begin with {}", s, REVERIE_ID_TO_NAME_KADKEY_PREFIX))
        }
    }
}



#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToPeerId(pub ReverieId);
const REVERIE_ID_TO_PEER_ID_KADKEY_PREFIX: &'static str = "reverie_to_peer_id";

impl ReverieIdToPeerId {
    pub fn to_string(&self) -> String {
        self.clone().into()
    }
}

impl Into<String> for ReverieIdToPeerId {
    fn into(self) -> String {
        format!("{}{}", REVERIE_ID_TO_PEER_ID_KADKEY_PREFIX, self.0)
    }
}

impl From<ReverieId> for ReverieIdToPeerId {
    fn from(reverie_id: ReverieId) -> Self {
        ReverieIdToPeerId(reverie_id)
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KademliaKey {
    VesselPeerId(VesselPeerId),
    ReverieIdToAgentName(ReverieIdToAgentName),
    Reverie(ReverieId),
    Unknown(String),
}

impl From<&str> for KademliaKey {
    fn from(s: &str) -> Self {
        match s {
            s if s.starts_with(VESSEL_KAD_KEY_PREFIX) => {
                KademliaKey::VesselPeerId(VesselPeerId::from_string(s).unwrap())
            }
            s if s.starts_with(REVERIE_ID_TO_NAME_KADKEY_PREFIX) => {
                KademliaKey::ReverieIdToAgentName(ReverieIdToAgentName::from_string(s).unwrap())
            }
            s if s.starts_with(REVERIE_ID_PREFIX) => {
                KademliaKey::Reverie(ReverieId::from(s))
            }
            _ => {
                KademliaKey::Unknown(s.to_string())
            }
        }
    }
}

impl From<&kad::Record> for KademliaKey {
    fn from(record: &kad::Record) -> Self {
        match std::str::from_utf8(record.key.as_ref()) {
            Ok(kad_key_str) => KademliaKey::from(kad_key_str),
            Err(e) => {
                tracing::error!("Error converting kad::Record to KademliaKey: {}", e);
                KademliaKey::Unknown(format!("{:?}", e))
            }
        }
    }
}

