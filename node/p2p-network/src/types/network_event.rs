use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use crate::network_events::peer_manager::AgentVesselTransferInfo;
use crate::types::{AgentNameWithNonce, FragmentNumber, RespawnId};
use crate::SendError;


#[derive(Debug)]
pub enum NetworkEvent {
    InboundCapsuleFragRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer_id: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
    RespawnRequest(
        AgentVesselTransferInfo,
    ),
    SaveKfragProviderRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        sender_peer_id: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
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