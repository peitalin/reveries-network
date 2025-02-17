use std::fmt::Display;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use crate::{short_peer_id, types::GossipTopic};
use umbral_pre::KeyFrag;


#[derive(Debug, Clone, Hash, Eq, PartialEq, Deserialize, Serialize)]
pub struct UmbralPeerId(pub String);

const UMBRAL_KEY_PREFIX: &'static str = "umbral_pubkey_";

impl UmbralPeerId {
    pub(crate) fn short_peer_id(&self) -> String {
        short_peer_id(&self)
    }
}

impl From<String> for UmbralPeerId {
    fn from(s: String) -> Self {
        if s.starts_with(UMBRAL_KEY_PREFIX) {
            let ssplit = s.split(UMBRAL_KEY_PREFIX).collect::<Vec<&str>>();
            UmbralPeerId(ssplit[1].to_string())
        } else {
            panic!("Invalid UmbralPeerId: {}. Must begin with {}", UMBRAL_KEY_PREFIX, s)
        }
    }
}

impl From<&str> for UmbralPeerId {
    fn from<'a>(s: &'a str) -> Self {
        if s.starts_with(UMBRAL_KEY_PREFIX) {
            let ssplit = s.split(UMBRAL_KEY_PREFIX).collect::<Vec<&str>>();
            UmbralPeerId(ssplit[1].to_string())
        } else {
            panic!("Invalid UmbralPeerId: {}. Must begin with {}", UMBRAL_KEY_PREFIX, s)
        }
    }
}

impl Into<String> for UmbralPeerId {
    fn into(self) -> String {
        format!("{}{}", UMBRAL_KEY_PREFIX, self.0)
    }
}

impl Display for UmbralPeerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", UMBRAL_KEY_PREFIX, self.0)
    }
}

impl From<PeerId> for UmbralPeerId {
    fn from(p: PeerId) -> Self {
        UmbralPeerId(p.to_string())
    }
}

impl From<&PeerId> for UmbralPeerId {
    fn from(p: &PeerId) -> Self {
        UmbralPeerId(p.to_string())
    }
}

impl Into<PeerId> for UmbralPeerId {
    fn into(self) -> PeerId {
        // slice off the UMBRAL_KEY_PREFIX first
        let umbral_peer_id_str = self.to_string();
        let peer_id_str = &umbral_peer_id_str.as_str()[UMBRAL_KEY_PREFIX.len()..];
        PeerId::from_str(peer_id_str)
            .expect("Error unwrapping UmbralPeerId to PeerId")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFragmentMessage {
    pub topic: GossipTopic,
    pub frag_num: usize,
    pub threshold: usize,
    pub total_frags: usize,
    pub alice_pk: umbral_pre::PublicKey,
    pub bob_pk: umbral_pre::PublicKey,
    pub verifying_pk: umbral_pre::PublicKey,
    // TODO: split data into private data for the MPC node, vs public data
    // private: kfrags, verify_pk, alice_pk, bob_pk -> store within the MPC node
    // public: capsules and ciphertexts -> store on Kademlia or in next Vessel(s)
    pub vessel_peer_id: PeerId,
    pub next_vessel_peer_id: PeerId,
    // kfrag
    pub kfrag: KeyFrag,
    pub capsule: umbral_pre::Capsule,
    pub ciphertext: Box<[u8]>
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapsuleFragmentMessage {
    pub frag_num: usize,
    pub threshold: usize,
    pub alice_pk: umbral_pre::PublicKey, // delegator
    pub bob_pk: umbral_pre::PublicKey, // delegatee (receiver)
    pub verifying_pk: umbral_pre::PublicKey, // key for anyone to verify the PRE capsules
    pub sender_peer_id: PeerId, // peer which holds and sends the kfrags
    pub vessel_peer_id: PeerId, // peer which houses the agent
    pub next_vessel_peer_id: PeerId, // next peer which houses the agent
    // cfrag
    pub cfrag: umbral_pre::CapsuleFrag,
    pub capsule: umbral_pre::Capsule,
    pub ciphertext: Box<[u8]>,
}

impl Display for CapsuleFragmentMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,
            "CapsuleFragmentMessage(frag_num: {}, alice_pk: {}, bob_pk: {}, verifying_pk, cfrag, ciphertext)",
            self.frag_num,
            self.alice_pk,
            self.bob_pk,
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize, Serialize)]
pub struct UmbralPublicKeyResponse {
    pub umbral_peer_id: UmbralPeerId,
    pub umbral_public_key: umbral_pre::PublicKey,
}

impl Display for UmbralPublicKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UmbralPublicKeyResponse({}, {})", self.umbral_peer_id, self.umbral_public_key)
    }
}