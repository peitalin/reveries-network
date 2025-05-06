#[path = "test_utils.rs"]
mod test_utils;

use std::{
    time::Duration,
    process::{Command, Stdio},
};
use color_eyre::{eyre::eyre, Result};
use jsonrpsee::core::client::ClientT;
use tokio::time;

use jsonrpsee::core::client::Client;

use p2p_network::types::NodeKeysWithVesselStatus;
use runtime::llm::read_agent_secrets;

// Node 1 is the bootstrap node
static BOOTSTRAP_PEER: &str = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";

use self::test_utils::{
    wait_for_rpc_server,
    init_test_logger,
};

/// Helper function to start a network of nodes and return clients
pub async fn start_test_network(
    num_nodes: usize,
    base_rpc_port: u16,
    base_listen_port: u16
) -> Result<Vec<Client>> {
    let bootstrap_url = &format!("/ip4/127.0.0.1/tcp/{}/p2p/{}", base_listen_port, BOOTSTRAP_PEER);
    // Prepare port lists
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|j| base_listen_port + j as u16).collect();

    // Start nodes (keep process handling for cleanup)
    let mut node_processes = Vec::with_capacity(num_nodes);
    for i in 0..num_nodes {
        let seed = i + 1; // Seeds start at 1
        let rpc_port = rpc_ports[i];
        let listen_port = listen_ports[i];
        let bootstrap = if i > 0 { Some(bootstrap_url) } else { None };

        let mut cmd = Command::new("cargo");
        cmd.current_dir("..")
            .args([
                "run", "--bin", "rpc", "--",
                "--secret-key-seed", &seed.to_string(),
                "--rpc-port", &rpc_port.to_string(),
                "--listen-address", &format!("/ip4/0.0.0.0/tcp/{}", listen_port),
            ])
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        if let Some(bootstrap_peer) = bootstrap {
            cmd.args(["--bootstrap-peers", bootstrap_peer]);
        }

        match cmd.spawn() {
            Ok(child) => {
                node_processes.push((rpc_port, child));
                println!("Started node {} on RPC port {} and listen port {}", seed, rpc_port, listen_port);
            }
            Err(e) => {
                tracing::warn!("Failed to spawn node {}: {}", seed, e);
                // Attempt basic cleanup before returning
                for (_, mut child) in node_processes {
                    let _ = child.kill();
                }
                return Err(eyre!("Failed to spawn node {}: {}", seed, e));
            }
        }
    }

    // Wait for RPC servers and collect clients
    let mut clients = Vec::with_capacity(node_processes.len());
    for (port, _child) in &node_processes {
        let client = wait_for_rpc_server(*port).await?;
        clients.push(client);
    }

    println!("All nodes started and ready");

    // Allow nodes to discover each other
    println!("Waiting longer for peer connections to be established...");
    time::sleep(Duration::from_secs(8)).await;

    Ok(clients)
}

/// Helper function to spawn an agent on a node and return the result
pub async fn spawn_agent_on_node(
    client: &Client,
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

    println!("Agent spawned on node: {:?}", spawn_result);

    // Allow time for fragment distribution
    time::sleep(Duration::from_millis(2000)).await;

    Ok(spawn_result)
}
