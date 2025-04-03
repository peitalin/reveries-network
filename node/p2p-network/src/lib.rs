#![allow(unused_imports, dead_code, unused_variables)]

pub mod behaviour;
pub mod create_network;
mod network_events;
pub mod node_client;
pub mod types;
pub mod utils;

use color_eyre::{Result, eyre::anyhow};
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use libp2p::{
    multiaddr,
    PeerId,
    Multiaddr,
};
use tokio::sync::oneshot::error::RecvError;


#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SendError(pub String);

impl std::error::Error for SendError {}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "SendError: {}", self.0)
    }
}

impl From<HashSet<PeerId>> for SendError {
    fn from(hset: HashSet<PeerId>) -> Self {
        SendError(anyhow!("sender.send(providers) err: {:?}", hset).to_string())
    }
}

impl From<umbral_pre::CapsuleFragVerificationError> for SendError {
    fn from(e: umbral_pre::CapsuleFragVerificationError) -> Self {
        SendError(e.to_string())
    }
}

impl From<(umbral_pre::KeyFragVerificationError, umbral_pre::KeyFrag)> for SendError {
    fn from(e: (umbral_pre::KeyFragVerificationError, umbral_pre::KeyFrag)) -> Self {
        SendError(e.0.to_string())
    }
}

impl From<RecvError> for SendError {
    fn from(e: RecvError) -> Self {
        SendError(e.to_string())
    }
}


impl From<Box<dyn std::error::Error + Send>> for SendError {
    fn from(e: Box<dyn std::error::Error + Send>) -> Self {
        SendError(e.to_string())
    }
}

pub fn short_peer_id<T: ToString>(peer_id: T) -> String {
    let peer_id_str = peer_id.to_string();
    if peer_id_str.len() < 10 {
        tracing::warn!("Not a valid PeerId")
    }
    let peer_id_end = peer_id_str.chars().rev().take(6).collect::<String>();
    let end = peer_id_end.chars().rev().collect::<String>();
    format!("PeerId(..{})", end)
}

pub fn read_file_from_path<'a>(path: &'a str) -> color_eyre::Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| color_eyre::eyre::anyhow!(e.to_string()))
}

pub trait TryPeerId {
    /// Tries convert `Self` into `PeerId`.
    fn try_into_peer_id(&self) -> Result<PeerId>;
}

impl TryPeerId for Multiaddr {
    fn try_into_peer_id(&self) -> Result<PeerId> {
        self.iter().last().and_then(|p| match p {
            multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
            multiaddr::Protocol::Dnsaddr(multiaddr) => {
                tracing::warn!(
                    "synchronous recursive dnsaddr resolution is not yet supported: {:?}",
                    multiaddr
                );
                None
            }
            _ => None,
        })
        .ok_or_else(|| anyhow!("dnsaddr error"))
    }
}

pub fn is_dialable(multiaddr: &Multiaddr) -> bool {
    // Check if the multiaddr is dialable
    match multiaddr.protocol_stack().next() {
        None => false,
        Some(protocol) => protocol != "p2p",
    }
}

// TODO: temporary convenience functions
pub fn get_node_name<'a>(peer_id: &libp2p::PeerId) -> &'a str {
    match peer_id.to_base58().as_str() {
        "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X" => "ALICE",
        "12D3KooWH3uVF6wv47WnArKHk5p6cvgCJEb74UTmxztmQDc298L3" => "BELLA",
        "12D3KooWQYhTNQdmr3ArTeUHRYzFg94BKyTkoWBDWez9kSCVe2Xo" => "CARL",
        "12D3KooWLJtG8fd2hkQzTn96MrLvThmnNQjTUFZwGEsLRz5EmSzc" => "DANY",
        "12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay" => "EMMA",
        "12D3KooWDMCQbZZvLgHiHntG1KwcHoqHPAxL37KvhgibWqFtpqUY" => "FRANK",
        "12D3KooWLnZUpcaBwbz9uD1XsyyHnbXUrJRmxnsMiRnuCmvPix67" => "GEMMA",
        "12D3KooWQ8vrERR8bnPByEjjtqV6hTWehaf8TmK7qR1cUsyrPpfZ" => "HANA",
        "12D3KooWNRk8VBuTJTYyTbnJC7Nj2UN5jij4dJMo8wtSGT2hRzRP" => "IAN",
        "12D3KooWFHNBwTxUgeHRcD3g4ieiXBmZGVyp6TKGWRKKEqYgCC1C" => "JACK",
        "12D3KooWHbEputWi1fJAxoYgmvvDe3yP7acTACqmXKGYwMgN2daQ" => "KASS",
        "12D3KooWCxnyz1JxC9y1RniRQVFe2cLaLHsYNc2SnXbM7yq5JBbJ" => "LARRY",
        _ => "Unnamed",
    }
}
