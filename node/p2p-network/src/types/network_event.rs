use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::network_events::peer_manager::peer_info::AgentVesselInfo;
use crate::types::{AgentNameWithNonce, FragmentNumber, ReverieId};
use crate::SendError;


#[derive(Debug)]
pub enum NetworkEvent {
    InboundCapsuleFragRequest {
        reverie_id: ReverieId,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
    RespawnRequest(
        AgentVesselInfo,
    ),
    SaveKfragProviderRequest {
        reverie_id: ReverieId,
        frag_num: usize,
        kfrag_provider_peer_id: PeerId,
        channel: ResponseChannel<FragmentResponseEnum>
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentRequestEnum {
    FragmentRequest(
        ReverieId,
        FragmentNumber,
        PeerId
    ),
    ProvidingFragment(
        ReverieId,
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