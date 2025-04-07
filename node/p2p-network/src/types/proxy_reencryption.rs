use std::fmt::Display;
use std::str::FromStr;
use color_eyre::{Result, eyre};
use serde::{Deserialize, Serialize};
use libp2p::{
    identity,
    PeerId
};
use crate::{short_peer_id, TryPeerId};
use crate::types::{
    AgentNameWithNonce,
    GossipTopic,
    ReverieId,
    ReverieType,
    ReverieKeyfrag,
};
use umbral_pre::KeyFrag;

use super::AgentVesselInfo;


#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct VesselPeerId(pub String);

const VESSEL_KAD_KEY_PREFIX: &'static str = "vessel_peer_id_";

impl VesselPeerId {
    pub(crate) fn short_peer_id(&self) -> String {
        short_peer_id(&self.0)
    }

    pub fn from_string<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(VESSEL_KAD_KEY_PREFIX) {
            let ssplit = s.split(VESSEL_KAD_KEY_PREFIX).collect::<Vec<&str>>();
            Ok(VesselPeerId(ssplit[1].to_string()))
        } else {
            Err(eyre::anyhow!("Invalid VesselPeerId: {}. Must begin with {}", s, VESSEL_KAD_KEY_PREFIX))
        }
    }

    pub fn is_kademlia_key<S: Into<String>>(s: S) -> Result<Self> {
        let s: String = s.into();
        if s.starts_with(VESSEL_KAD_KEY_PREFIX) {
            let ssplit = s.split(VESSEL_KAD_KEY_PREFIX).collect::<Vec<&str>>();
            Ok(VesselPeerId(ssplit[1].to_string()))
        } else {
            Err(eyre::anyhow!("Invalid VesselPeerId kademlia key: {}. Must begin with {}", s, VESSEL_KAD_KEY_PREFIX))
        }
    }
}

impl Into<String> for VesselPeerId {
    fn into(self) -> String {
        format!("{}{}", VESSEL_KAD_KEY_PREFIX, self.0)
    }
}

impl Display for VesselPeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", VESSEL_KAD_KEY_PREFIX, self.0)
    }
}

impl From<PeerId> for VesselPeerId {
    fn from(p: PeerId) -> Self {
        VesselPeerId(p.to_string())
    }
}

impl From<&PeerId> for VesselPeerId {
    fn from(p: &PeerId) -> Self {
        VesselPeerId(p.to_string())
    }
}

impl TryPeerId for VesselPeerId {
    fn try_into_peer_id(&self) -> Result<PeerId> {
        // slice off the VESSEL_KAD_KEY_PREFIX first
        let umbral_peer_id_str = self.to_string();
        let peer_id_str = &umbral_peer_id_str.as_str()[VESSEL_KAD_KEY_PREFIX.len()..];
        PeerId::from_str(peer_id_str)
            .map_err(|e| eyre::anyhow!(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFragmentMessage2 {
    pub reverie_keyfrag: ReverieKeyfrag,
    pub frag_num: usize,
    pub threshold: usize,
    pub total_frags: usize,
    pub source_peer_id: PeerId,
    pub target_peer_id: PeerId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFragmentMessage {
    pub topic: GossipTopic,
    pub reverie_id: ReverieId,
    pub frag_num: usize,
    pub threshold: usize,
    pub total_frags: usize,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
    // TODO: split data into private data for the MPC node, vs public data
    // private: kfrags, verify_pk, alice_pk, bob_pk -> store within the MPC node
    // public: capsules and ciphertexts -> store on Kademlia or in next Vessel(s)
    pub vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
    // kfrag
    pub kfrag: KeyFrag,
    pub capsule: umbral_pre::Capsule,
    pub ciphertext: Box<[u8]>
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleFragmentMessage {
    pub agent_name: Option<AgentNameWithNonce>,
    pub frag_num: usize,
    pub threshold: usize,
    pub alice_pk: umbral_pre::PublicKey, // delegator
    pub bob_pk: umbral_pre::PublicKey, // delegatee (receiver)
    pub verifying_pk: umbral_pre::PublicKey, // key for anyone to verify the PRE capsules
    pub kfrag_provider_peer_id: PeerId, // peer which holds and sends the kfrags
    pub vessel_peer_id: PeerId, // peer which houses the agent
    pub next_vessel_peer_id: PeerId, // next peer which houses the agent
    // cfrag
    pub cfrag: umbral_pre::CapsuleFrag,
    pub capsule: umbral_pre::Capsule,
    pub ciphertext: Box<[u8]>,
}

impl Display for CapsuleFragmentMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,
            "CapsuleFragmentMessage(frag_num: {}, alice_pk: {}, bob_pk: {}, verifying_pk, cfrag, ciphertext)",
            self.frag_num,
            self.alice_pk,
            self.bob_pk,
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct NodeVesselWithStatus {
    pub peer_id: PeerId,
    pub umbral_public_key: umbral_pre::PublicKey,
    pub agent_vessel_info: Option<AgentVesselInfo>,
    pub vessel_status: VesselStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum VesselStatus {
    // nodes that should never host agent
    NeverVessel,
    // nodes that are able to host agent, but empty at the moment
    EmptyVessel,
    // nodes that are currently hosting an agent
    ActiveVessel
}

impl Display for NodeVesselWithStatus  {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeVesselWithStatus({}, {})", self.peer_id, self.umbral_public_key)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct RespawnId {
    prev_agent: AgentNameWithNonce,
    next_vessel_peer_id: libp2p::PeerId,
}

impl RespawnId {
    pub fn new(
        prev_agent: &AgentNameWithNonce,
        next_vessel_peer_id: &libp2p::PeerId,
    ) -> Self {
        Self {
            prev_agent: prev_agent.clone(),
            next_vessel_peer_id: next_vessel_peer_id.clone(),
        }
    }

    pub fn get_request_id(&self) -> String {
        let respawn_id = hex::encode(format!("{}{}", self.prev_agent, self.next_vessel_peer_id));
        respawn_id
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedVesselStatus {
    pub node_vessel_status: NodeVesselWithStatus,
    pub signature: Vec<u8>,
}

impl SignedVesselStatus {
    pub fn new(status: NodeVesselWithStatus, id_keys: &identity::Keypair) -> Result<Self> {
        let status_bytes = serde_json::to_vec(&status)?;
        let signature = id_keys.sign(&status_bytes)?;
        Ok(Self {
            node_vessel_status: status,
            signature: signature.to_vec(),
        })
    }

    pub fn verify(&self, publisher_id: &PeerId) -> Result<()> {
        let status_bytes = serde_json::to_vec(&self.node_vessel_status)?;

        // Convert PeerId bytes to PublicKey
        let public_key = identity::PublicKey::try_decode_protobuf(&publisher_id.to_bytes())
            .map_err(|e| eyre::anyhow!("Failed to decode public key: {}", e))?;

        if !public_key.verify(&status_bytes, &self.signature) {
            return Err(eyre::anyhow!("Invalid vessel status signature"));
        }
        Ok(())
    }
}