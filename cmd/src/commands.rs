use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;


#[derive(Parser, Serialize, Deserialize, Clone, Debug)]
#[clap(name = "libp2p-client")]
pub(crate) struct Cmd {

    #[clap(long)]
    pub rpc_server_address:  SocketAddr,

    #[clap(subcommand)]
    pub argument: CliArgument,
}

#[derive(Debug, Parser, Clone, Deserialize, Serialize)]
pub enum CliArgument {
    Broadcast {
        #[clap(long)]
        agent_name: String,
        #[clap(long)]
        agent_nonce: usize,
        #[clap(long)]
        shares: usize,
        #[clap(long)]
        threshold: usize,
    },
    Respawn {
        #[clap(long)]
        agent_name: String,
        #[clap(long)]
        agent_nonce: usize,
    },
    GetKfragPeers {
        #[clap(long)]
        agent_name: String,
        #[clap(long)]
        agent_nonce: usize,
    },
    TopicSwitch {
        #[clap(long)]
        next_agent_name: String,
        #[clap(long)]
        next_agent_nonce: usize,
        #[clap(long)]
        total_frags: usize,
        #[clap(long)]
        threshold: usize,
        #[clap(long)]
        prev_agent_name: String,
        #[clap(long)]
        prev_agent_nonce: usize,
        #[clap(long)]
        peer_id: String,
    },
}
