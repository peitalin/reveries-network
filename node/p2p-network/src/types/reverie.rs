
use color_eyre::{eyre::anyhow, Result};
use serde::{Deserialize, Serialize};
use umbral_pre::Capsule;
use crate::utils::reverie_id;
use crate::types::AgentNameWithNonce;

pub type ReverieId = String;

/// An encrypted memory module, used by an agent
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Reverie {
    pub id: ReverieId,
    pub reverie_type: String,
    pub description: String,
    pub umbral_capsule: Capsule,
    pub umbral_ciphertext: Box<[u8]>,
}

impl Reverie {
    pub fn new(
        description: String,
        reverie_type: String,
        capsule: Capsule,
        ciphertext: Box<[u8]>
    ) -> Self {
        Self {
            id: reverie_id(),
            reverie_type: reverie_type,
            description: description,
            umbral_capsule: capsule,
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
