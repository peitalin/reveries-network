use clap::Parser;
use serde::{Deserialize, Serialize};
use libp2p::Multiaddr;

#[derive(Parser, Debug, Serialize, Deserialize)]
#[clap(name = "libp2p example")]
pub struct Opt {
    /// Fixed value to generate deterministic peer ID.
    #[clap(long)]
    pub secret_key_seed: Option<usize>,

    #[clap(long)]
    pub rpc_port: Option<usize>,

     // Allow comma-separated values
    #[clap(long, value_delimiter = ',')]
    pub listen_address: Vec<Multiaddr>,

    // Format: "peer_id@ip:port"
    #[clap(long, value_delimiter = ',')]
    pub bootstrap_peers: Vec<String>,

    // Format: "docker compose -f <docker_compose_file_path> <other_args>"
    #[clap(long)]
    pub docker_compose_cmd: Option<String>,
}
