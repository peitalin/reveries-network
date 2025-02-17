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
