mod test_utils;

use std::time::Duration;
use color_eyre::Result;
use tokio::time;
use std::net::SocketAddr;
use jsonrpsee::core::client::ClientT;
use serde_json::Value;
use std::process::Command;
use tracing::{info, warn};
use std::collections::HashSet;

use p2p_network::types::NodeVesselWithStatus;
use runtime::llm::read_agent_secrets;
use rpc::rpc_client::create_rpc_client;
use telemetry::init_logger;

use test_utils::{
    wait_for_rpc_server,
    CleanupGuard,
};


#[tokio::test]
#[serial_test::serial]
async fn test_agent_spawn_and_fragments() -> Result<()> {

    // Initialize telemetry for better logging
    let _ = color_eyre::install();
    init_logger(telemetry::LoggerConfig {
        show_log_level: true,
        show_path: true,
        ..Default::default()
    });

    // Define the ports we'll be using in this test
    let rpc_ports = vec![8001, 8002, 8003, 8004, 8005];
    let listen_ports = vec![9001, 9002, 9003, 9004, 9005];
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up ports on instantiation and when it goes out of scope
    let _guard = CleanupGuard::with_ports(all_test_ports);

    let bootstrap_peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
    let bootstrap_url = &format!("/ip4/127.0.0.1/tcp/9001/p2p/{}", bootstrap_peer);
    let seeds = vec![1, 2, 3, 4, 5];

    // Start nodes in separate processes
    let node_configs = vec![
        (seeds[0], rpc_ports[0], listen_ports[0], None),
        (seeds[1], rpc_ports[1], listen_ports[1], Some(bootstrap_url)),
        (seeds[2], rpc_ports[2], listen_ports[2], Some(bootstrap_url)),
        (seeds[3], rpc_ports[3], listen_ports[3], Some(bootstrap_url)),
        (seeds[4], rpc_ports[4], listen_ports[4], Some(bootstrap_url)),
    ];

    // Start all processes and track their RPC ports
    let mut active_rpc_ports = Vec::new();
    for (seed, rpc_port, listen_port, bootstrap) in node_configs {
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
                // Don't track the process, just record the port
                active_rpc_ports.push(rpc_port);
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

    // Spawn an agent on the first node
    let threshold = 2;
    let total_frags = 3;
    let secret_key_seed = 1;
    let agent_secrets_json = read_agent_secrets(secret_key_seed);

    let spawn_result: NodeVesselWithStatus = clients[0]
        .request(
            "spawn_agent",
            jsonrpsee::rpc_params![
                agent_secrets_json,
                threshold,
                total_frags
            ],
        )
        .await?;

    // Allow time for fragment distribution
    time::sleep(Duration::from_millis(2000)).await;

    // Get node state from all nodes
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
                        if let Some(cfrag) = cfrag_data.get("cfrag") {
                            all_cfrags.push(cfrag_data.clone());
                        }
                    }
                }
            }
        }
    }

    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    println!("\n==> Found {} cfrags for {} nodes\n", total_cfrags, active_rpc_ports.len());
    println!("\nAll cfrags:\n{}", serde_json::to_string_pretty(&all_cfrags)?);
    println!("\nVerifying unique and common fields across cfrags...");

    // Verify that all cfrags have the same values for common fields
    if all_cfrags.len() > 1 {
        let first_cfrag = &all_cfrags[0];

        // Fields that should be the same across all cfrags
        let common_fields = [
            "alice_pk",
            "bob_pk",
            "threshold",
            "next_vessel_peer_id",
            "verifying_pk",
            "vessel_peer_id",
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