
use color_eyre::{eyre::anyhow, Result};
use serde::{Deserialize, Serialize};
use umbral_pre::Capsule;
use libp2p::PeerId;
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
}



#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct AgentReverieId(pub String);

const AGENT_REVERIE_ID_KAD_KEY_PREFIX: &'static str = "agent_reverie_id_";

impl Into<String> for AgentReverieId {
    fn into(self) -> String {
        format!("{}{}", AGENT_REVERIE_ID_KAD_KEY_PREFIX, self.0)
    }
}

impl From<AgentNameWithNonce> for AgentReverieId {
    fn from(agent_name_nonce: AgentNameWithNonce) -> Self {
        AgentReverieId(agent_name_nonce.to_string())
    }
}

impl AgentReverieId {
    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(AGENT_REVERIE_ID_KAD_KEY_PREFIX) {
            let ssplit = s.split(AGENT_REVERIE_ID_KAD_KEY_PREFIX).collect::<Vec<&str>>();
            Ok(AgentReverieId(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid AgentReverieId: {}. Must begin with {}", s, AGENT_REVERIE_ID_KAD_KEY_PREFIX))
        }
    }

    pub fn to_string(&self) -> String {
        self.clone().into()
    }

    pub fn is_kademlia_key<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(AGENT_REVERIE_ID_KAD_KEY_PREFIX) {
            let ssplit = s.split(AGENT_REVERIE_ID_KAD_KEY_PREFIX).collect::<Vec<&str>>();
            Ok(AgentReverieId(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid AgentReverieId kademlia key: {}. Must begin with {}", s, AGENT_REVERIE_ID_KAD_KEY_PREFIX))
        }
    }
}
