mod chat;
mod network_event;
mod proxy_reencryption;
mod gossip_topic;

pub use chat::*;
pub use network_event::*;
pub use proxy_reencryption::*;
pub use gossip_topic::*;
use serde::{Serialize, Deserialize};


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