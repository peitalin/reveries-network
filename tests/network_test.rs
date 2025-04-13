mod test_utils;

use std::{
    time::Duration,
    net::SocketAddr,
    process::Command,
    collections::HashSet,
};
use color_eyre::Result;
use jsonrpsee::core::{
    client::ClientT,
    params::ArrayParams,
};
use tokio::time;
use serde_json::Value;
use tracing::{info, warn};

use p2p_network::{
    types::{NodeVesselWithStatus, ReverieNameWithNonce},
    node_client::RestartReason,
};
use runtime::llm::read_agent_secrets;
use rpc::rpc_client::create_rpc_client;
use telemetry::init_logger;

use test_utils::{
    wait_for_rpc_server,
    CleanupGuard,
};

/// Helper function to start a network of nodes and return clients
async fn start_test_network(
    num_nodes: usize,
    base_rpc_port: u16,
    base_listen_port: u16
) -> Result<Vec<jsonrpsee::core::client::Client>> {
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
    time::sleep(Duration::from_millis(2000)).await;

    Ok(clients)
}

/// Helper function to spawn an agent on a node and return the result
async fn spawn_agent_on_node(
    client: &jsonrpsee::core::client::Client,
    threshold: usize,
    total_frags: usize,
    seed: usize
) -> Result<NodeVesselWithStatus> {
    let agent_secrets_json = read_agent_secrets(seed);

    let spawn_result: NodeVesselWithStatus = client
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

/// Helper function to collect fragments from all nodes
async fn collect_fragments(clients: &[jsonrpsee::core::client::Client]) -> Result<Vec<Value>> {
    let mut all_cfrags = Vec::new();

    for (i, client) in clients.iter().enumerate() {
        let state: Value = client
            .request(
                "get_node_state",
                jsonrpsee::rpc_params![],
            )
            .await?;

        info!("Node {} state: {}", i + 1, serde_json::to_string_pretty(&state)?);

        // Extract cfrags from state
        if let Some(peer_manager) = state.get("peer_manager") {
            if let Some(cfrags_summary) = peer_manager.get("1_cfrags_summary") {
                if let Some(array) = cfrags_summary.as_array() {
                    for cfrag_data in array {
                        if let Some(_) = cfrag_data.get("cfrag") {
                            all_cfrags.push(cfrag_data.clone());
                        }
                    }
                }
            }
        }
    }

    Ok(all_cfrags)
}

/// Helper function to trigger node failure
async fn trigger_node_failure(client: &jsonrpsee::core::client::Client) -> Result<RestartReason> {
    let result: RestartReason = client
        .request(
            "trigger_node_failure",
            jsonrpsee::rpc_params![],
        )
        .await?;

    info!("Node failure triggered: {:?}", result);

    Ok(result)
}

/// Helper function to wait for agent respawning on a node
async fn wait_for_agent_respawn(
    client: &jsonrpsee::core::client::Client,
    timeout_secs: u64
) -> Result<ReverieNameWithNonce> {
    // Wait for the respawning process to occur
    info!("Waiting for respawning process...");

    // Check state every few seconds until we see respawning or timeout
    let respawn_timeout = Duration::from_secs(timeout_secs);
    let start_time = std::time::Instant::now();

    while start_time.elapsed() < respawn_timeout {
        // Wait before checking
        time::sleep(Duration::from_secs(2)).await;
        info!("Checking for agent respawning...");

        match client.request::<Value, _>("get_node_state", jsonrpsee::rpc_params![]).await {
            Ok(state) => {
                if let Some(vessel_agent) = state.get("_agent_in_vessel") {
                    if let Some(agent_name_nonce) = vessel_agent.get("agent_name_nonce") {
                        println!("[Test] Node taken over as vessel: {}",
                              serde_json::to_string_pretty(vessel_agent)?);

                        let agent_str = agent_name_nonce.to_string();
                        return Ok(ReverieNameWithNonce::from(agent_str));
                    }
                }
                println!("[Test] Agent not yet respawned, waiting...");
            },
            Err(e) => {
                println!("[Test] Error checking node state: {}", e);
                // Continue waiting, as this might be a temporary error
            }
        }
    }

    // Timeout reached, no agent found
    Err(color_eyre::eyre::eyre!("Agent not respawned within the timeout period"))
}

#[tokio::test]
#[serial_test::serial]
async fn test_agent_spawn_and_fragments() -> Result<()> {
    // Initialize telemetry for better logging
    test_utils::init_test_logger();

    // Define the ports we'll be using in this test
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;

    // Create port lists for the CleanupGuard
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up ports on instantiation and when it goes out of scope
    let _guard = CleanupGuard::with_ports(all_test_ports);

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;

    // Spawn an agent on the first node
    let threshold = 2;
    let total_frags = 3;
    let secret_key_seed = 1;
    let _spawn_result = spawn_agent_on_node(&clients[0], threshold, total_frags, secret_key_seed).await?;

    // Get node state from all nodes and collect fragments
    let all_cfrags = collect_fragments(&clients).await?;

    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    info!("\n==> Found {} cfrags for {} nodes\n", total_cfrags, rpc_ports.len());
    info!("\nAll cfrags:\n{}", serde_json::to_string_pretty(&all_cfrags)?);
    info!("\nVerifying unique and common fields across cfrags...");

    // Verify that all cfrags have the same values for common fields
    if all_cfrags.len() > 1 {
        let first_cfrag = &all_cfrags[0];

        // Fields that should be the same across all cfrags
        let common_fields = [
            "source_pubkey",
            "target_pubkey",
            "threshold",
            "source_verifying_pubkey",
            "target_verifying_pubkey",
        ];

        for field in common_fields {
            let expected_value = first_cfrag["cfrag"][field].clone();
            for cfrag_data in &all_cfrags[1..] {
                assert_eq!(
                    cfrag_data["cfrag"][field],
                    expected_value,
                    "Field '{}' should be the same across all cfrags",
                    field
                );
            }
        }

        // Verify unique fields are different
        let mut seen_cfrags = HashSet::new();
        let mut seen_kfrag_provider_peer_ids = HashSet::new();

        for cfrag_data in &all_cfrags {

            // Check cfrag uniqueness
            let cfrag_value = cfrag_data["cfrag"]["cfrag"].as_str().unwrap();
            assert!(
                seen_cfrags.insert(cfrag_value),
                "Duplicate cfrag found"
            );

            // Check kfrag_provider_peer_id uniqueness
            let kfrag_provider_peer_id = cfrag_data["cfrag"]["kfrag_provider_peer_id"].as_str().unwrap();
            assert!(
                seen_kfrag_provider_peer_ids.insert(kfrag_provider_peer_id),
                "Duplicate kfrag_provider_peer_id found"
            );
        }
    }

    // Cleanup happens automatically via CleanupGuard drop
    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn test_agent_respawn_after_failure() -> Result<()> {
    // Initialize telemetry for better logging
    test_utils::init_test_logger();

    // Define the ports we'll be using in this test
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 6;
    // let rpc_ports = vec![8001, 8002, 8003, 8004, 8005, 8006];
    // let listen_ports = vec![9001, 9002, 9003, 9004, 9005, 9006];

    // Create port lists for the CleanupGuard
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up when it goes out of scope
    let _guard = CleanupGuard::with_ports(all_test_ports);

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;

    // Spawn an agent on the first node
    let threshold = 2;
    let total_frags = 3;
    let secret_key_seed = 1;
    let _spawn_result = spawn_agent_on_node(&clients[0], threshold, total_frags, secret_key_seed).await?;

    // Verify fragments are distributed properly
    let all_cfrags = collect_fragments(&clients).await?;

    // Make sure we have the expected number of fragments
    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    println!("\n[Test] Found {} fragments distributed across nodes", total_cfrags);

    // PART 2: Test agent respawning after node failure
    println!("[Test] Testing agent respawning after node failure");

    // Trigger a simulated node failure on the first node
    println!("[Test] Triggering simulated failure on node1 (port 8001)");
    let _failure_result = trigger_node_failure(&clients[0]).await?;

    // Wait for node2 to respawn the agent (timeout after 30 seconds)
    let respawned_agent = wait_for_agent_respawn(&clients[1], 30).await?;
    println!("[Test] Agent respawned: {:?}", respawned_agent);

    // Verify that the agent was successfully respawned
    let expected_agent = ReverieNameWithNonce("auron".to_string(), 1);
    // Verify that the agent was successfully respawned
    println!("[Test] Agent respawned: {}", respawned_agent);
    println!("[Test] {} == {}: {}", respawned_agent, expected_agent, respawned_agent == expected_agent);
    assert_eq!(respawned_agent, expected_agent, "Respawned agent doesn't match expected agent");

    // Cleanup happens automatically via CleanupGuard drop
    Ok(())
}