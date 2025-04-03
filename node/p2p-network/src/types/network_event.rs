use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::network_events::peer_manager::peer_info::AgentVesselInfo;
use crate::types::{AgentNameWithNonce, FragmentNumber};
use crate::SendError;


#[derive(Debug)]
pub enum NetworkEvent {
    InboundCapsuleFragRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
    RespawnRequest(
        AgentVesselInfo,
    ),
    SaveKfragProviderRequest {
        agent_name_nonce: AgentNameWithNonce,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId,
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