use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use libp2p::core::Multiaddr;

#[derive(Parser, Debug, Serialize, Deserialize)]
#[clap(name = "libp2p example")]
pub struct Opt {
    /// Fixed value to generate deterministic peer ID.
    #[clap(long)]
    pub secret_key_seed: Option<u8>,

    #[clap(long)]
    pub listen_address: Option<Multiaddr>,

    #[clap(long, value_parser, num_args = 1.., value_delimiter = ',')]
    pub topics: Option<Vec<String>>,

    #[clap(long)]
    pub rpc_port: Option<u16>,
}
