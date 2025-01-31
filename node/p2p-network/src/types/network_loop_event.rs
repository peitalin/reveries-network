use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::AgentName;

#[derive(Debug)]
pub enum NetworkLoopEvent {
    InboundCfragRequest {
        agent_name: String,
        frag_num: Option<usize>,
        channel: ResponseChannel<FragmentResponse>
    },

    Respawn(AgentName, PeerId),

    ReBroadcastKfrags(AgentName),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRequest(pub String, pub Option<usize>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentResponse(pub Vec<u8>);
