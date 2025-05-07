mod rpc_server;
mod rpc_client;
mod commands;

use color_eyre::{Result, eyre::anyhow, eyre::Error};
use clap::Parser;
use libp2p::{Multiaddr, multiaddr::Protocol};
use std::env;

use commands::Opt;
use p2p_network::create_network;
use rpc_server::run_server;


#[tokio::main]
async fn main() -> Result<()> {

    // Initialize color_eyre
    color_eyre::install()?;
    // Only initialize logger if not running in test environment
    if env::var("TEST_ENV").is_err() {
        telemetry::init_logger(telemetry::LoggerConfig {
            show_log_level: true,
            show_path: true,
            show_time: false,
            show_crate_name: false,
            ..Default::default()
        });
    }

    let opt = Opt::parse();

    let bootstrap_nodes: Vec<(String, Multiaddr)> = opt.bootstrap_peers
        .iter()
        .filter_map(|addr_str| {
            // Parse multiaddr format: /ip4/node1/tcp/port/p2p/peer_id
            let addr: Multiaddr = addr_str.parse().ok()?;
            // Extract peer ID from the multiaddr
            let peer_id = addr.iter().find_map(|p| {
                match p {
                    Protocol::P2p(peer_id) => Some(peer_id.to_string()),
                    _ => None,
                }
            })?;

            Some((peer_id, addr))
        })
        .collect();

    // Create the network and start the node client
    let node_client = create_network::new(
        opt.secret_key_seed,
        opt.listen_address,
        bootstrap_nodes
    ).await?;

    // then run an RPC server if provided an RPC port
    if let Some(port) = opt.rpc_port {
        // await to keep the server running
        run_server(port, node_client).await?;
    }

    Ok(())
}


