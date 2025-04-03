use std::time::Duration;
use color_eyre::Result;
use libp2p::multiaddr::Multiaddr;
use tokio::time;
use tokio::sync::oneshot;
use std::net::SocketAddr;
use jsonrpsee::core::client::ClientT;
use serde_json::Value;
use std::process::{Command, Child};
use tracing::{info, warn};
use std::collections::HashSet;

// Import crates from the project
use p2p_network::create_network;
use p2p_network::node_client::NodeCommand;
use p2p_network::types::NodeVesselWithStatus;
use runtime::llm::read_agent_secrets;
use rpc::rpc_client::create_rpc_client;
use telemetry::init_logger;

async fn wait_for_rpc_server(port: u16) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut retries = 10;

    while retries > 0 {
        match create_rpc_client(&addr).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!("Waiting for RPC server on port {}: {}", port, e);
                tokio::time::sleep(Duration::from_secs(1)).await;
                retries -= 1;
            }
        }
    }
    Err(color_eyre::eyre::eyre!("RPC server failed to start"))
}

struct TestNode {
    process: Child,
    rpc_port: u16,
}

impl Drop for TestNode {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

#[tokio::test]
async fn test_node_listening_on_specific_port() -> Result<()> {
    // Install color_eyre for better error reporting
    let _ = color_eyre::install();

    // Set up a specific port for testing
    let port = 9101;
    let listen_address: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", port).parse()?;

    // Create a node with a specific seed for deterministic testing
    let seed = Some(42);
    let bootstrap_nodes = vec![];

    let (mut node_client, _network_events_receiver, network_event_loop) =
        create_network::new(seed, bootstrap_nodes).await?;

    // Start the network event loop in the background
    let event_loop_handle = tokio::task::spawn(network_event_loop.listen_for_network_events());

    // Start listening on the specified address
    node_client.start_listening_to_network(Some(listen_address.clone())).await?;

    // Give the node a moment to start listening
    time::sleep(Duration::from_millis(300)).await;

    // Get the node's listening addresses
    let (sender, receiver) = oneshot::channel();
    node_client.command_sender.send(NodeCommand::GetListeningAddresses { sender }).await?;
    let listening_addresses = receiver.await?;

    // Check if the node is listening on the expected port
    let is_listening_on_port = listening_addresses.iter().any(|addr| {
        addr.to_string().contains(&format!("/tcp/{}", port))
    });

    assert!(is_listening_on_port, "Node is not listening on port {}", port);

    // Clean up
    event_loop_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_two_nodes_connecting() -> Result<()> {
    // We don't reinstall color_eyre to avoid conflicts

    // Set up specific ports for testing
    let port1 = 9201;
    let port2 = 9202;

    let listen_address1: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", port1).parse()?;
    let listen_address2: Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", port2).parse()?;

    // Create the first node with a specific seed
    let (mut node1_client, _node1_events_receiver, node1_event_loop) =
        create_network::new(Some(101), vec![]).await?;

    // Start the first node's event loop
    let event_loop1_handle = tokio::task::spawn(node1_event_loop.listen_for_network_events());

    // Start listening on the first address
    node1_client.start_listening_to_network(Some(listen_address1.clone())).await?;

    // Give the first node time to start listening
    time::sleep(Duration::from_millis(300)).await;

    // Get the first node's peer ID
    let peer1_id = node1_client.node_id.peer_id;

    // Create a bootstrap multiaddr for the second node to connect to the first
    let bootstrap_addr = format!("{}/p2p/{}", listen_address1, peer1_id);
    let bootstrap_multiaddr: Multiaddr = bootstrap_addr.parse()?;

    // Create bootstrap nodes list for the second node
    let bootstrap_nodes = vec![(peer1_id.to_string(), bootstrap_multiaddr)];

    // Create the second node with different seed and bootstrap to first node
    let (mut node2_client, _node2_events_receiver, node2_event_loop) =
        create_network::new(Some(102), bootstrap_nodes).await?;

    // Start the second node's event loop
    let event_loop2_handle = tokio::task::spawn(node2_event_loop.listen_for_network_events());

    // Start listening on the second address
    node2_client.start_listening_to_network(Some(listen_address2.clone())).await?;

    // Subscribe both nodes to the same topic to encourage connection
    let test_topic = "test_connection".to_string();
    node1_client.subscribe_topics(vec![test_topic.clone()]).await?;
    node2_client.subscribe_topics(vec![test_topic.clone()]).await?;

    // Give nodes time to discover each other and establish connection
    time::sleep(Duration::from_secs(3)).await;

    // Check node 1's listening addresses
    let (sender1, receiver1) = oneshot::channel();
    node1_client.command_sender.send(NodeCommand::GetListeningAddresses { sender: sender1 }).await?;
    let listening_addresses1 = receiver1.await?;

    // Check node 2's listening addresses
    let (sender2, receiver2) = oneshot::channel();
    node2_client.command_sender.send(NodeCommand::GetListeningAddresses { sender: sender2 }).await?;
    let listening_addresses2 = receiver2.await?;

    // Verify that both nodes are running on different ports
    let is_listening1 = listening_addresses1.iter().any(|addr| {
        addr.to_string().contains(&format!("/tcp/{}", port1))
    });

    let is_listening2 = listening_addresses2.iter().any(|addr| {
        addr.to_string().contains(&format!("/tcp/{}", port2))
    });

    assert!(is_listening1, "Node 1 is not listening on port {}", port1);
    assert!(is_listening2, "Node 2 is not listening on port {}", port2);

    // Get node 2's peer ID
    let peer2_id = node2_client.node_id.peer_id;

    // Verify that nodes are connected to each other
    // Get node 1's connected peers
    let (sender1, receiver1) = oneshot::channel();
    node1_client.command_sender.send(NodeCommand::GetConnectedPeers { sender: sender1 }).await?;
    let connected_peers1 = receiver1.await?;

    // Get node 2's connected peers
    let (sender2, receiver2) = oneshot::channel();
    node2_client.command_sender.send(NodeCommand::GetConnectedPeers { sender: sender2 }).await?;
    let connected_peers2 = receiver2.await?;

    // Check that node1 is connected to node2
    assert!(
        connected_peers1.contains(&peer2_id),
        "Node 1 is not connected to Node 2"
    );

    // Check that node2 is connected to node1
    assert!(
        connected_peers2.contains(&peer1_id),
        "Node 2 is not connected to Node 1"
    );

    // Clean up
    event_loop1_handle.abort();
    event_loop2_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_agent_spawn_and_fragments() -> Result<()> {
    // Initialize telemetry for better logging
    let _ = color_eyre::install();
    init_logger(telemetry::LoggerConfig {
        show_log_level: true,
        show_path: true,
        ..Default::default()
    });

    // Start 3 nodes in separate processes
    let nodes = vec![
        TestNode {
            process: Command::new("cargo")
                .current_dir("..")  // Move up to workspace root
                .args([
                    "run", "--bin", "rpc", "--",
                    "--secret-key-seed", "1",
                    "--rpc-port", "8001",
                    "--listen-address", "/ip4/0.0.0.0/tcp/9001"
                ])
                .spawn()
                .expect("Failed to start node 1"),
            rpc_port: 8001,
        },
        TestNode {
            process: Command::new("cargo")
                .current_dir("..")  // Move up to workspace root
                .args([
                    "run", "--bin", "rpc", "--",
                    "--secret-key-seed", "2",
                    "--rpc-port", "8002",
                    "--listen-address", "/ip4/0.0.0.0/tcp/9002",
                    "--bootstrap-peers", "/ip4/127.0.0.1/tcp/9001/p2p/12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X"
                ])
                .spawn()
                .expect("Failed to start node 2"),
            rpc_port: 8002,
        },
        TestNode {
            process: Command::new("cargo")
                .current_dir("..")  // Move up to workspace root
                .args([
                    "run", "--bin", "rpc", "--",
                    "--secret-key-seed", "3",
                    "--rpc-port", "8003",
                    "--listen-address", "/ip4/0.0.0.0/tcp/9003",
                    "--bootstrap-peers", "/ip4/127.0.0.1/tcp/9001/p2p/12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X"
                ])
                .spawn()
                .expect("Failed to start node 3"),
            rpc_port: 8003,
        },
        TestNode {
            process: Command::new("cargo")
                .current_dir("..")  // Move up to workspace root
                .args([
                    "run", "--bin", "rpc", "--",
                    "--secret-key-seed", "4",
                    "--rpc-port", "8004",
                    "--listen-address", "/ip4/0.0.0.0/tcp/9004",
                    "--bootstrap-peers", "/ip4/127.0.0.1/tcp/9001/p2p/12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X"
                ])
                .spawn()
                .expect("Failed to start node 4"),
            rpc_port: 8004,
        }
    ];

    // Wait for all RPC servers to be ready
    for node in &nodes {
        wait_for_rpc_server(node.rpc_port).await?;
    }

    info!("All nodes started and ready");

    // Create RPC clients for each node
    let mut clients = Vec::new();
    for node in &nodes {
        let addr = SocketAddr::from(([127, 0, 0, 1], node.rpc_port));
        let client = create_rpc_client(&addr).await?;
        clients.push(client);
    }

    // Allow nodes to discover each other and establish connections
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Spawn an agent on the first node
    let total_frags = 3;
    let threshold = 2;
    let secret_key_seed = 1;
    let agent_secrets_json = read_agent_secrets(secret_key_seed);

    let spawn_result: NodeVesselWithStatus = clients[0]
        .request(
            "spawn_agent",
            jsonrpsee::rpc_params![
                agent_secrets_json,
                total_frags,
                threshold
            ],
        )
        .await?;

    info!("Agent spawned successfully");

    // Allow time for fragment distribution
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Get node state from all nodes
    let mut has_fragments = false;
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
                            has_fragments = true;
                            all_cfrags.push(cfrag_data.clone());
                        }
                    }
                }
            }
        }
    }

    let total_cfrags = all_cfrags.len();
    assert!(total_cfrags == 3, "Wrong number of key fragments found");
    println!("\n==> Found {} cfrags for {} nodes\n", total_cfrags, nodes.len());
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
        let mut seen_frag_nums = HashSet::new();
        let mut seen_cfrags = HashSet::new();
        let mut seen_kfrag_provider_peer_ids = HashSet::new();

        for cfrag_data in &all_cfrags {
            // Check frag_num uniqueness
            let frag_num = cfrag_data["cfrag"]["frag_num"].as_u64().unwrap();
            assert!(
                seen_frag_nums.insert(frag_num),
                "Duplicate frag_num found: {}",
                frag_num
            );

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

    // TestNode's Drop trait will handle cleanup
    Ok(())
}