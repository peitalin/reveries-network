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
    /// The signature is verified before giving access to the fragment,
    /// ensuring that only authorized parties can decrypt fragments.
    ///
    /// The verification process:
    /// 1. The target vessel signs the keccak256 hash of the reverie_id using its UmbralKey.signer
    /// 2. The fragment holder verifies the signature against the target vessel's public key (bob_pk)
    /// 3. If verification passes, the fragment is returned; otherwise, the request is rejected
    GetFragmentRequest(
        ReverieId,
        umbral_pre::Signature,  // signature over keccak256(reverie_id)
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