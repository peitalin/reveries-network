use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::types::AgentNameWithNonce;
use crate::SendError;


#[derive(Debug)]
pub enum NetworkLoopEvent {
    InboundCfragRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: Option<usize>,
        sender_peer: PeerId,
        channel: ResponseChannel<FragmentResponse>
    },
    RespawnRequired {
        agent_name_nonce: AgentNameWithNonce, // with prev agent_nonce
        total_frags: usize,
        prev_peer_id: PeerId
    },
    ReBroadcastKfrags(AgentNameWithNonce),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentRequest(
    pub AgentNameWithNonce,
    pub Option<usize>,
    pub PeerId
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentResponse(
    pub Result<Vec<u8>, SendError>,
);
