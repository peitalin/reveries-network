use clap::Parser;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use p2p_network::types::{ReverieId, ReverieType, SignatureType};


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

    GetKfragProviders {
        #[clap(long)]
        agent_name: String,
        #[clap(long)]
        agent_nonce: usize,
    },

    SpawnAgent {
        #[clap(long)]
        threshold: usize,
        #[clap(long)]
        total_frags: usize,
        // #[clap(long)]
        // secret_key_seed: usize,
    },

    TriggerNodeFailure,

    GetNodeStates {
        #[clap(long, value_parser, num_args = 1.., value_delimiter = ',')]
        ports: Vec<String>,
    },

    Websocket,

    SubscribeHeartbeat,

    #[clap(name = "spawn-memory-reverie")]
    SpawnMemoryReverie {
        /// JSON containing memory secrets
        #[clap(long)]
        memory_secrets: serde_json::Value,

        /// Minimum number of fragments needed for reconstruction
        #[clap(long)]
        threshold: usize,

        /// Total number of fragments to create
        #[clap(long)]
        total_frags: usize,
    },

    #[clap(name = "execute-with-memory-reverie")]
    ExecuteWithMemoryReverie {
        /// The ID of the reverie to execute
        #[clap(long)]
        reverie_id: String,

        /// The type of the reverie (Memory, Agent, SovereignAgent)
        #[clap(long)]
        reverie_type: String,

        /// Optional signature for verification (in hex format)
        #[clap(long)]
        signature: SignatureType,
    },
}
