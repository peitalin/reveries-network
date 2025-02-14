pub mod behaviour;
pub mod create_network;
mod network_events;
pub mod node_client;
pub mod types;

use serde::{Deserialize, Serialize};



#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SendError(pub String);

impl std::error::Error for SendError {}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "SendError: {}", self.0)
    }
}

pub fn short_peer_id(peer_id: &libp2p::PeerId) -> String {
    let peer_id_str = peer_id.to_string();
    if peer_id_str.len() < 10 {
        panic!("Not a valid PeerId")
    }
    let peer_id_end = peer_id_str.chars().rev().take(6).collect::<String>();
    // let start = &peer_id_str[..4];
    let end = peer_id_end.chars().rev().collect::<String>();
    format!("PeerId(..{})", end)
}

pub fn read_file_from_path<'a>(path: &'a str) -> color_eyre::Result<Vec<u8>> {
    std::fs::read(path)
        .map_err(|e| color_eyre::eyre::anyhow!(e.to_string()))
}

pub fn get_node_name(peer_id: &libp2p::PeerId) -> String {
    match peer_id.to_base58().as_str() {
        "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X" => "ALICE".to_string(),
        "12D3KooWH3uVF6wv47WnArKHk5p6cvgCJEb74UTmxztmQDc298L3" => "BELLA".to_string(),
        "12D3KooWQYhTNQdmr3ArTeUHRYzFg94BKyTkoWBDWez9kSCVe2Xo" => "CARL".to_string(),
        "12D3KooWLJtG8fd2hkQzTn96MrLvThmnNQjTUFZwGEsLRz5EmSzc" => "DANY".to_string(),
        "12D3KooWSHj3RRbBjD15g6wekV8y3mm57Pobmps2g2WJm6F67Lay" => "EMMA".to_string(),
        _ => "Unnamed".to_string(),
    }
}

pub fn make_node_name<'a>(seed: usize) -> &'a str {
    match seed {
        1 => "ALICE",
        2 => "BELLA",
        3 => "CARL",
        4 => "DANY",
        5 => "EMMA",
        _ => "Unnamed",
    }
}