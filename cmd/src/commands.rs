use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;


#[derive(Parser, Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Cmd {
    #[clap(long)]
    pub rpc_server_address:  SocketAddr,

    #[clap(long)]
    pub agent_name: String,

    #[clap(long)]
    pub shares: usize,

    #[clap(long)]
    pub threshold: usize,
}
