use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use crate::types::{
    ReverieNameWithNonce,
    ReverieId,
    ReverieType,
};
use super::heartbeat_data;

#[derive(Clone, Debug)]
pub(crate) struct PeerInfo {
    pub peer_id: PeerId,
    pub heartbeat_data: heartbeat_data::HeartBeatData,
    pub agent_vessel: Option<AgentVesselInfo>,
    pub client_version: Option<String>,
}

impl PeerInfo {
    pub fn new(peer_id: PeerId, heartbeat_avg_window: u32) -> Self {
        Self {
            peer_id,
            heartbeat_data: heartbeat_data::HeartBeatData::new(heartbeat_avg_window),
            agent_vessel: None,
            client_version: None,
        }
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentVesselInfo {
    pub reverie_id: ReverieId,
    pub reverie_type: ReverieType,
    pub total_frags: usize,
    pub threshold: usize,
    pub current_vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
}
