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
    ReverieNameWithNonce,
    ReverieId,
    ReverieType,
    ReverieKeyfrag,
    Reverie,
};
use umbral_pre::KeyFrag;

use super::AgentVesselInfo;


#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct VesselPeerId(pub String);

pub const VESSEL_KAD_KEY_PREFIX: &'static str = "vessel_peer_id_";

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

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct NodeVesselWithStatus {
    pub peer_id: PeerId,
    pub umbral_public_key: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
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
    prev_agent: ReverieNameWithNonce,
    next_vessel_peer_id: libp2p::PeerId,
}

impl RespawnId {
    pub fn new(
        prev_agent: &ReverieNameWithNonce,
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