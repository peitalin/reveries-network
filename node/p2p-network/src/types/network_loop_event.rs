use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use color_eyre::eyre;
use crate::types::{AgentName, AgentNonce};
use crate::SendError;

#[derive(Debug)]
pub enum NetworkLoopEvent {
    InboundCfragRequest {
        agent_name: String,
        agent_nonce: usize,
        frag_num: Option<usize>,
        channel: ResponseChannel<FragmentResponse>
    },
    RespawnRequired {
        agent_name: AgentName,
        agent_nonce: AgentNonce, // prev agent_nonce
        total_frags: usize,
        prev_peer_id: PeerId
    },
    ReBroadcastKfrags(
        AgentName,
        AgentNonce
    ),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRequest(pub String, pub usize, pub Option<usize>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentResponse(pub Result<Vec<u8>, SendError>);
