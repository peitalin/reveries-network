use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::types::{AgentName, AgentNonce};

#[derive(Debug)]
pub enum NetworkLoopEvent {
    InboundCfragRequest {
        agent_name: String,
        agent_nonce: usize,
        frag_num: Option<usize>,
        channel: ResponseChannel<FragmentResponse>
    },
    Respawn(
        AgentName,
        AgentNonce,
        PeerId // prev_vessel_peer_id
    ),
    ReBroadcastKfrags(
        AgentName,
        AgentNonce
    ),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRequest(pub String, pub usize, pub Option<usize>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentResponse(pub Vec<u8>);
