use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::types::{AgentNameWithNonce, FragmentNumber};
use crate::SendError;


#[derive(Debug)]
pub enum NetworkLoopEvent {
    InboundCfragRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
    RespawnRequiredRequest {
        agent_name_nonce: AgentNameWithNonce, // with prev agent_nonce
        total_frags: usize,
        prev_peer_id: PeerId
    },
    SaveKfragProviderRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
    ReBroadcastKfrags(AgentNameWithNonce),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentRequestEnum {
    FragmentRequest(
        AgentNameWithNonce,
        FragmentNumber,
        PeerId
    ),
    ProvidingFragment(
        AgentNameWithNonce,
        FragmentNumber,
        PeerId
    )
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentResponseEnum {
    FragmentResponse(
        Result<Vec<u8>, SendError>,
    ),
    KfragProviderAck
}