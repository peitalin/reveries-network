
use color_eyre::{eyre::anyhow, Result};
use serde::{Deserialize, Serialize};
use umbral_pre::Capsule;
use libp2p::{PeerId, kad};
use crate::utils::reverie_id;
use crate::types::AgentNameWithNonce;

pub type ReverieId = String;

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
    pub alice_pk: umbral_pre::PublicKey, // source
    pub bob_pk: umbral_pre::PublicKey,   // target
    pub verifying_pk: umbral_pre::PublicKey,
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReverieCapsulefrag {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub frag_num: usize,
    pub threshold: usize,
    pub umbral_capsule_frag: Vec<u8>,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
    pub kfrag_provider_peer_id: PeerId,
}


#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum ReverieType {
    Agent,
    Retrieval,
    Tools,
    Memory,
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
            umbral_capsule: serde_json::to_vec(&capsule).expect(""),
            umbral_ciphertext: ciphertext
        }
    }

    pub fn get_capsule(&self) -> Result<umbral_pre::Capsule> {
        serde_json::from_slice(&self.umbral_capsule)
            .map_err(|e| anyhow!("Error deserializing Capsule: {}", e))
    }
}



#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToAgentName(pub String);

const REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX: &'static str = "reverie_to_agent_name";


impl Into<String> for ReverieIdToAgentName {
    fn into(self) -> String {
        format!("{}{}", REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX, self.0)
    }
}

impl From<AgentNameWithNonce> for ReverieIdToAgentName {
    fn from(agent_name_nonce: AgentNameWithNonce) -> Self {
        ReverieIdToAgentName(agent_name_nonce.to_string())
    }
}

impl ReverieIdToAgentName {
    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToAgentName(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToAgentName: {}. Must begin with {}", s, REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX))
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
        if s.starts_with(REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToAgentName(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToAgentName kademlia key: {}. Must begin with {}", s, REVERIE_ID_TO_AGENTNAME_KADKEY_PREFIX))
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
