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
    PeerIdToNodeStatusKey,
    ReverieId,
    VerifyingKey,
    PEER_ID_TO_NODE_STATUS,
};


pub trait KademliaKeyTrait {
    fn to_kad_key(&self) -> kad::RecordKey;
    fn to_string(&self) -> String;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KademliaKey {
    PeerIdToNodeStatusKey(PeerIdToNodeStatusKey),
    ReverieIdToNameKey(ReverieIdToNameKey),
    ReverieIdToReverie(ReverieId),
    ReverieIdToKfragProvidersKey(ReverieIdToKfragProvidersKey),
    Unknown(String),
}

impl From<&str> for KademliaKey {
    fn from(s: &str) -> Self {
        match s {
            // peerId -> vessel status queries
            s if s.starts_with(PEER_ID_TO_NODE_STATUS) => {
                KademliaKey::PeerIdToNodeStatusKey(PeerIdToNodeStatusKey::from_string(s).unwrap())
            }
            // reverieId -> Reverie queries
            s if s.starts_with(REVERIE_ID_PREFIX) => {
                KademliaKey::ReverieIdToReverie(ReverieId::from(s))
            }
            // reverieId -> agent name queries
            s if s.starts_with(REVERIE_ID_TO_NAME_KADKEY_PREFIX) => {
                KademliaKey::ReverieIdToNameKey(ReverieIdToNameKey::from_string(s).unwrap())
            }
            // reverieId -> kfrag providers queries
            s if s.starts_with(REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX) => {
                KademliaKey::ReverieIdToKfragProvidersKey(ReverieIdToKfragProvidersKey::from_string(s).unwrap())
            }
            _ => {
                KademliaKey::Unknown(s.to_string())
            }
        }
    }

    // fn to_kad_key(&self) -> kad::RecordKey {
    //     match self {
    //         KademliaKey::PeerIdToNodeStatusKey(key) => key.to_kad_key(),
    //         KademliaKey::ReverieIdToNameKey(key) => key.to_kad_key(),
    //         KademliaKey::ReverieIdToReverie(key) => key.to_kad_key(),
    //         KademliaKey::ReverieIdToKfragProvidersKey(key) => key.to_kad_key(),
    //         KademliaKey::Unknown(key) => {
    //             tracing::error!("Unknown KademliaKey: {}", key);
    //             kad::RecordKey::new(&key)
    //         }
    //     }
    // }
}

impl From<&kad::RecordKey> for KademliaKey {
    fn from(record_key: &kad::RecordKey) -> Self {
        match std::str::from_utf8(record_key.as_ref()) {
            Ok(kad_key_str) => KademliaKey::from(kad_key_str),
            Err(e) => {
                tracing::error!("Error converting kad::Record to KademliaKey: {}", e);
                KademliaKey::Unknown(format!("{:?}", e))
            }
        }
    }
}


pub const REVERIE_ID_TO_NAME_KADKEY_PREFIX: &'static str = "reverie_id_to_name";

#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToNameKey(pub String);

impl From<ReverieNameWithNonce> for ReverieIdToNameKey {
    fn from(agent_name_nonce: ReverieNameWithNonce) -> Self {
        ReverieIdToNameKey(agent_name_nonce.to_string())
    }
}
impl KademliaKeyTrait for ReverieIdToNameKey {
    fn to_string(&self) -> String {
        format!("{}{}", REVERIE_ID_TO_NAME_KADKEY_PREFIX, self.0)
    }
    fn to_kad_key(&self) -> kad::RecordKey {
        kad::RecordKey::new(&self.to_string())
    }
}
impl ReverieIdToNameKey {
    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(REVERIE_ID_TO_NAME_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_NAME_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToNameKey(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToNameKey: {}. Must begin with {}", s, REVERIE_ID_TO_NAME_KADKEY_PREFIX))
        }
    }
}


const REVERIE_ID_TO_PEER_ID_KADKEY_PREFIX: &'static str = "reverie_id_to_peer_id";

#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToPeerId(pub ReverieId);

impl KademliaKeyTrait for ReverieIdToPeerId {
    fn to_string(&self) -> String {
        format!("{}{}", REVERIE_ID_TO_PEER_ID_KADKEY_PREFIX, self.0)
    }
    fn to_kad_key(&self) -> kad::RecordKey {
        kad::RecordKey::new(&self.to_string())
    }
}

impl From<ReverieId> for ReverieIdToPeerId {
    fn from(reverie_id: ReverieId) -> Self {
        ReverieIdToPeerId(reverie_id)
    }
}


const REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX: &'static str = "reverie_id_to_kfrag_providers_";

#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct ReverieIdToKfragProvidersKey(pub ReverieId);

impl KademliaKeyTrait for ReverieIdToKfragProvidersKey {
    fn to_string(&self) -> String {
        format!("{}{}", REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX, self.0)
    }
    fn to_kad_key(&self) -> kad::RecordKey {
        kad::RecordKey::new(&self.to_string())
    }
}

impl From<ReverieId> for ReverieIdToKfragProvidersKey {
    fn from(reverie_id: ReverieId) -> Self {
        ReverieIdToKfragProvidersKey(reverie_id)
    }
}

impl ReverieIdToKfragProvidersKey {
    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX) {
            let ssplit = s.split(REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX).collect::<Vec<&str>>();
            Ok(ReverieIdToKfragProvidersKey(ssplit[1].to_string()))
        } else {
            Err(anyhow!("Invalid ReverieIdToKfragProvidersKey: {}. Must begin with {}", s, REVERIE_ID_TO_KFRAG_PROVIDERS_KADKEY_PREFIX))
        }
    }
}