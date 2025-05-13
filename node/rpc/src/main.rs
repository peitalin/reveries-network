mod rpc_server;
mod rpc_client;
mod commands;

use color_eyre::Result;
use clap::Parser;
use libp2p::{Multiaddr, multiaddr::Protocol};
use std::env;
use rpc_server::run_server;
use tokio::time::Duration;
use tracing::{info, error};

use commands::Opt;
use p2p_network::create_network;


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
        bootstrap_nodes,
    ).await?;

    let mut rpc_server_running = false;

    // then run an RPC server if provided an RPC port
    if let Some(port) = opt.rpc_port {
        info!("Starting RPC server on port {}...", port);
        run_server(port, node_client.clone()).await?;
        info!("RPC server running at: {}", port);
        rpc_server_running = true;

        // Now, after RPC server is initiated, attempt to start Docker services if specified
        if let Some(ref docker_cmd) = opt.docker_compose_cmd {
            info!("RPC server initiated, now attempting to start Docker services...");
            // Adding a small, pragmatic delay to give the OS/network stack a moment
            // for the RPC port to become fully listenable before llm-proxy tries to connect.
            tokio::time::sleep(Duration::from_secs(1)).await;

            let p2p_node_rpc_url = format!("http://host.docker.internal:{}", port);
            if let Err(e) = node_client.start_docker_service(
                p2p_node_rpc_url,
                docker_cmd.clone()
            ).await {
                error!("Failed to start docker service (e.g., llm-proxy): {}", e);
                // Optionally, decide if this is a fatal error for the main application
            }
        } else {
            info!("No docker_compose_cmd provided, skipping Docker service start.");
        }
    } else {
        info!("No RPC port provided, RPC server will not be started. Docker services also skipped if they depend on RPC.");
    }

    if rpc_server_running {
        info!("p2p-node running with RPC server. Press Ctrl+C to exit.");
        tokio::signal::ctrl_c().await?;
        info!("Ctrl+C received, shutting down p2p-node.");
    } else {
        info!("p2p-node setup complete (no RPC server). Application will now exit if no other foreground tasks are running.");
        // If there are other non-RPC related long-running tasks, they would keep it alive.
        // Otherwise, it will exit here.
    }

    Ok(())
}


