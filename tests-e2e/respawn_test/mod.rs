#[path = "../utils_docker.rs"]
mod utils_docker;
#[path = "../utils_network.rs"]
mod utils_network;

use std::{
    time::Duration,
    collections::{HashSet, HashMap}
};
use color_eyre::Result;
use jsonrpsee::core::{
    client::ClientT,
    params::ArrayParams,
};
use jsonrpsee::http_client::HttpClient;
use tokio::time;
use serde_json::Value;
use tracing::{info, warn};

use p2p_network::types::{
    ReverieNameWithNonce,
    NodeKeysWithVesselStatus,
};
use p2p_network::node_client::RestartReason;
use runtime::llm::read_agent_secrets;

use utils_network::{
    P2PNodeCleanupGuard,
    start_test_network,
    Port,
};

/// Helper function to spawn an agent on a node and return the result
pub async fn spawn_agent_on_node(
    client: &HttpClient,
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

/// Helper function to collect fragments from all nodes
async fn collect_fragments(clients: &HashMap<Port, HttpClient>) -> Result<Vec<Value>> {
    let mut all_cfrags = Vec::new();

    for (port, client) in clients.iter() {
        let state: Value = client
            .request(
                "get_node_state",
                jsonrpsee::rpc_params![],
            )
            .await?;

        info!("Node(port: {}) state: {}", port, serde_json::to_string_pretty(&state)?);

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
async fn trigger_node_failure(client: &HttpClient) -> Result<RestartReason> {
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
    client: &HttpClient,
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
                println!("json: {}", serde_json::to_string_pretty(&state)?);
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
pub async fn test_agent_spawn_and_fragments() -> Result<()> {

    let base_rpc_port = 9901;
    let base_listen_port = 9001;
    let num_nodes = 5;
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up ports on instantiation and when it goes out of scope
    let _guard = P2PNodeCleanupGuard::with_ports(all_test_ports);

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;

    // Spawn an agent on the first node
    let threshold = 2;
    let total_frags = 3;
    let secret_key_seed = 1;
    let _spawn_result = spawn_agent_on_node(
        &clients[&9901],
        threshold,
        total_frags,
        secret_key_seed
    ).await?;

    // Get node state from all nodes and collect fragments
    let all_cfrags = collect_fragments(&clients).await?;

    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    info!("\n==> Found {} cfrags for {} nodes\n", total_cfrags, rpc_ports.len());
    info!("All cfrags:\n{}", serde_json::to_string_pretty(&all_cfrags)?);
    info!("Verifying unique and common fields across cfrags...");

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
pub async fn test_agent_respawn_after_failure() -> Result<()> {

    let base_rpc_port = 9901;
    let base_listen_port = 9001;
    let num_nodes = 6;
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up when it goes out of scope
    let _guard = P2PNodeCleanupGuard::with_ports(all_test_ports);

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;

    // Spawn an agent on the first node
    let threshold = 2;
    let total_frags = 3;
    let secret_key_seed = 1;
    let _spawn_result = spawn_agent_on_node(&clients[&9901], threshold, total_frags, secret_key_seed).await?;

    // Verify fragments are distributed properly
    let all_cfrags = collect_fragments(&clients).await?;

    // Make sure we have the expected number of fragments
    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    println!("\n[Test] Found {} fragments distributed across nodes", total_cfrags);

    // PART 2: Test agent respawning after node failure
    println!("[Test] Testing agent respawning after node failure");

    // Trigger a simulated node failure on the first node
    println!("[Test] Triggering simulated failure on node1 (port: 9901)");
    let _failure_result = trigger_node_failure(&clients[&9901]).await?;

    // Wait for node2 to respawn the agent (timeout after 30 seconds)
    let respawned_agent = wait_for_agent_respawn(&clients[&9902], 30).await?;
    println!("[Test] Agent respawned on node2 (port: 9902): {:?}", respawned_agent);

    // Verify that the agent was successfully respawned
    let expected_agent = ReverieNameWithNonce("auron".to_string(), 1);
    // Verify that the agent was successfully respawned
    println!("[Test] Agent respawned: {}", respawned_agent);
    println!("[Test] {} == {}: {}", respawned_agent, expected_agent, respawned_agent == expected_agent);
    assert_eq!(respawned_agent, expected_agent, "Respawned agent doesn't match expected agent");

    // Cleanup happens automatically via CleanupGuard drop
    Ok(())
}