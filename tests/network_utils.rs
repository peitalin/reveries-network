#[path = "test_utils.rs"]
mod test_utils;

use std::{
    time::Duration,
    net::SocketAddr,
    process::Command,
};
use color_eyre::Result;
use jsonrpsee::core::client::ClientT;
use tokio::time;
use tracing::{info, warn};

use p2p_network::types::NodeKeysWithVesselStatus;
use runtime::llm::read_agent_secrets;
use rpc::rpc_client::create_rpc_client;

use self::test_utils::{
    wait_for_rpc_server,
    CleanupGuard,
    init_test_logger,
};

/// Helper function to start a network of nodes and return clients
pub async fn start_test_network(
    num_nodes: usize,
    base_rpc_port: u16,
    base_listen_port: u16
) -> Result<Vec<jsonrpsee::core::client::Client>> {

    // initialize the logger once for all tests
    init_test_logger();

    let bootstrap_peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
    let bootstrap_url = &format!("/ip4/127.0.0.1/tcp/{}/p2p/{}", base_listen_port, bootstrap_peer);

    // Prepare port lists
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();

    // Start nodes
    let mut active_rpc_ports = Vec::new();
    for i in 0..num_nodes {
        let seed = i + 1; // Seeds start at 1
        let rpc_port = rpc_ports[i];
        let listen_port = listen_ports[i];
        let bootstrap = if i > 0 { Some(bootstrap_url) } else { None };

        let mut cmd = Command::new("cargo");
        cmd.current_dir("..")  // Move up to workspace root
            .args([
                "run", "--bin", "rpc", "--",
                "--secret-key-seed", &seed.to_string(),
                "--rpc-port", &rpc_port.to_string(),
                "--listen-address", &format!("/ip4/0.0.0.0/tcp/{}", listen_port),
            ]);

        if let Some(bootstrap_peer) = bootstrap {
            cmd.args(["--bootstrap-peers", bootstrap_peer]);
        }

        match cmd.spawn() {
            Ok(_) => {
                active_rpc_ports.push(rpc_port);
                info!("Started node {} on RPC port {} and listen port {}", seed, rpc_port, listen_port);
            }
            Err(e) => {
                warn!("Failed to start node {}: {}", seed, e);
                return Err(color_eyre::eyre::eyre!("Failed to start node {}: {}", seed, e));
            }
        }
    }

    // Wait for all RPC servers to be ready
    for &port in &active_rpc_ports {
        wait_for_rpc_server(port).await?;
    }

    info!("All nodes started and ready");

    // Create RPC clients for each node
    let mut clients = Vec::new();
    for &port in &active_rpc_ports {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let client = create_rpc_client(&addr).await?;
        clients.push(client);
    }

    // Allow nodes to discover each other and establish connections
    // Increase timeout to give more time for peer discovery
    info!("Waiting for peer connections to be established...");
    time::sleep(Duration::from_secs(2)).await;

    Ok(clients)
}

/// Helper function to spawn an agent on a node and return the result
pub async fn spawn_agent_on_node(
    client: &jsonrpsee::core::client::Client,
    threshold: usize,
    total_frags: usize,
    seed: usize
) -> Result<NodeKeysWithVesselStatus> {
    let agent_secrets_json = read_agent_secrets(seed);

    let spawn_result: NodeKeysWithVesselStatus = client
        .request(
            "spawn_agent",
            jsonrpsee::rpc_params![
                agent_secrets_json,
                threshold,
                total_frags
            ],
        )
        .await?;

    info!("Agent spawned on node: {:?}", spawn_result);

    // Allow time for fragment distribution
    time::sleep(Duration::from_millis(2000)).await;

    Ok(spawn_result)
}