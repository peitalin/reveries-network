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
        shares: usize,
        #[clap(long)]
        threshold: usize,
    },
    Respawn {
        #[clap(long)]
        agent_name: String,
    },
    GetAgentKfragPeers {
        #[clap(long)]
        agent_name: String,
    }
}
