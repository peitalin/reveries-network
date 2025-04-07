use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::network_events::peer_manager::peer_info::AgentVesselInfo;
use crate::types::{
    AgentNameWithNonce,
    FragmentNumber,
    ReverieId,
    Reverie,
    ReverieKeyfrag,
    KeyFragmentMessage2,
};
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentRequestEnum {
    /// TargetVessel requests KeyFrags to decrypt a Reverie
    GetFragmentRequest(
        ReverieId,
        FragmentNumber,
        PeerId
    ),
    /// KeyFrag holder notifies TargetVessel it holds a KeyFrag
    ProvidingFragmentRequest(
        ReverieId,
        FragmentNumber,
        PeerId
    ),
    /// Encryptor sends node a KeyFrag to save
    SaveFragmentRequest(
        KeyFragmentMessage2,
        Option<AgentNameWithNonce>
    )
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentResponseEnum {
    FragmentResponse(
        Result<Vec<u8>, SendError>,
    ),
    KfragProviderAck
}