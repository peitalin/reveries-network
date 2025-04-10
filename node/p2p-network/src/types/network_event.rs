use libp2p::request_response::ResponseChannel;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use crate::types::{
    AgentNameWithNonce,
    AgentVesselInfo,
    FragmentNumber,
    ReverieId,
    Reverie,
    ReverieKeyfrag,
    ReverieKeyfragMessage,
    ReverieMessage,
};
use crate::SendError;


#[derive(Debug)]
pub enum NetworkEvent {
    RespawnRequest(
        AgentVesselInfo,
    ),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentRequestEnum {
    /// TargetVessel requests Capsule Frags to decrypt a Reverie
    GetFragmentRequest(
        ReverieId,
        PeerId
    ),
    /// Encryptor sends provider nodes a KeyFrag to save
    SaveFragmentRequest(
        ReverieKeyfragMessage,
        Option<AgentNameWithNonce>
    ),
    /// KeyFrag holder notifies target vessel it holds a KeyFrag
    ProvidingFragmentRequest(
        ReverieId,
        FragmentNumber,
        PeerId
    ),
    /// Encryptor sends node a Reverie/Ciphertext to save
    SaveCiphertextRequest(
        ReverieMessage,
        Option<AgentNameWithNonce>
    ),
    /// Mark Respawn Complete
    MarkRespawnCompleteRequest {
        prev_reverie_id: ReverieId,
        prev_peer_id: PeerId,
        prev_agent_name: AgentNameWithNonce
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FragmentResponseEnum {
    GetFragmentResponse(
        Result<Vec<u8>, SendError>,
    ),

    SaveFragmentResponse,

    ProvidingFragmentResponse,

    SaveCiphertextResponse,

    MarkRespawnCompleteResponse,
}