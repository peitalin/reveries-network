
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
    VerifyingKey,
    PEER_ID_TO_NODE_STATUS,
};

pub type ReverieId = String;

/// An encrypted memory module, used by an agent
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Reverie {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub description: String,
    pub threshold: usize,
    pub total_frags: usize,
    pub target_public_key: umbral_pre::PublicKey,
    pub verifying_public_key: VerifyingKey,
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
    pub source_pubkey: umbral_pre::PublicKey,           // source_pubkey
    pub source_verifying_pubkey: umbral_pre::PublicKey, // source verifying key
    pub target_pubkey: umbral_pre::PublicKey,           // target_pubkey
    pub target_verifying_pubkey: VerifyingKey,          // target verifying key
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverieMessage {
    pub reverie: Reverie,
    pub source_peer_id: PeerId,
    pub target_peer_id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverieKeyfragMessage {
    pub reverie_keyfrag: ReverieKeyfrag,
    pub source_peer_id: PeerId,
    pub target_peer_id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct ReverieCapsulefrag {
    pub id: ReverieId,
    pub reverie_type: ReverieType,
    pub frag_num: usize,
    pub threshold: usize,
    pub umbral_capsule_frag: Vec<u8>,
    pub source_pubkey: umbral_pre::PublicKey,           // source_pubkey
    pub source_verifying_pubkey: umbral_pre::PublicKey, // source verifying key
    pub target_pubkey: umbral_pre::PublicKey,           // target_pubkey
    pub target_verifying_pubkey: VerifyingKey,          // target verifying key
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
        target_public_key: umbral_pre::PublicKey,
        verifying_public_key: VerifyingKey,
        capsule: umbral_pre::Capsule,
        ciphertext: Box<[u8]>
    ) -> Self {
        Self {
            id: reverie_id(),
            reverie_type: reverie_type,
            description: description,
            threshold: threshold,
            total_frags: total_frags,
            target_public_key: target_public_key,
            verifying_public_key: verifying_public_key,
            umbral_capsule: serde_json::to_vec(&capsule).expect("Failed to serialize capsule"),
            umbral_ciphertext: ciphertext
        }
    }

    pub fn encode_capsule(&self) -> Result<umbral_pre::Capsule> {
        serde_json::from_slice(&self.umbral_capsule)
            .map_err(|e| anyhow!("Error deserializing Capsule: {}", e))
    }
}


