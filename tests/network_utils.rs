#[path = "test_utils.rs"]
mod test_utils;

use std::{
    time::Duration,
    net::SocketAddr,
    process::{Command, Stdio},
};
use color_eyre::Result;
use jsonrpsee::core::client::ClientT;
use tokio::time;
use tracing::{info, warn, error};

use p2p_network::types::NodeKeysWithVesselStatus;
use runtime::llm::read_agent_secrets;
use rpc::rpc_client::create_rpc_client;

use self::test_utils::{
    wait_for_rpc_server,
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
                println!("Started node {} on RPC port {} and listen port {}", seed, rpc_port, listen_port);
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

    println!("All nodes started and ready");

    // Create RPC clients for each node
    let mut clients = Vec::new();
    for &port in &active_rpc_ports {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let client = create_rpc_client(&addr).await?;
        clients.push(client);
    }

    // Allow nodes to discover each other and establish connections
    // Increase timeout to give more time for peer discovery
    println!("Waiting for peer connections to be established...");
    time::sleep(Duration::from_secs(3)).await;

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

    println!("Agent spawned on node: {:?}", spawn_result);

    // Allow time for fragment distribution
    time::sleep(Duration::from_millis(2000)).await;

    Ok(spawn_result)
}

pub async fn start_python_server() -> Result<std::process::Child> {
    // Use a line separator instead of string multiplication
    let separator = "================================================================================";
    println!("Starting Python FastAPI server...");

    // Determine Python command based on platform
    let python_cmd = if cfg!(windows) { "python" } else { "python3" };
    let script_path = "agents/python/execute_with_memories/main.py";

    // Start the Python server process
    let mut cmd = Command::new(python_cmd);
    cmd.current_dir("..")  // Move up to workspace root
        .arg(script_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    println!("Executing command: {:?}", cmd);

    // Spawn the process and handle potential errors
    let mut python_server = match cmd.spawn() {
        Ok(child) => {
            println!("Python process started with PID: {}", child.id());
            child
        },
        Err(e) => {
            return Err(color_eyre::eyre::eyre!("Failed to start Python server: {}", e));
        }
    };

    // Configure health check parameters
    let server_port = 8000;
    let retry_delay = Duration::from_millis(500);
    let max_retries = 20;

    println!("Waiting up to {:.1} seconds for FastAPI server on port {}...",
          retry_delay.as_secs_f64() * max_retries as f64, server_port);

    // Health check loop
    for attempt in 1..=max_retries {
        // Check if the process has exited prematurely
        if let Ok(Some(status)) = python_server.try_wait() {
            // If the process has exited, collect and return any output
            let mut error_msg = format!("Python server process exited with status: {}", status);
            append_process_output(&mut error_msg, &mut python_server);
            return Err(color_eyre::eyre::eyre!(error_msg));
        }

        // Attempt to connect to the server's health endpoint
        match reqwest::get(format!("http://localhost:{}/health", server_port)).await {
            Ok(response) if response.status().is_success() => {
                // Server is running - try to get PID from health response
                if let Ok(json) = response.json::<serde_json::Value>().await {
                    if let Some(pid) = json.get("pid").and_then(|p| p.as_u64()) {
                        println!("{}\nPython FastAPI server is running on port {} with PID {}\n{}",
                              separator, server_port, pid, separator);
                    } else {
                        println!("{}\nPython FastAPI server is running on port {}\n{}",
                              separator, server_port, separator);
                    }
                } else {
                    println!("{}\nPython FastAPI server is running on port {}\n{}",
                          separator, server_port, separator);
                }
                return Ok(python_server);
            },
            Ok(response) => {
                warn!("Attempt {}/{}: FastAPI server responded with status: {}",
                      attempt, max_retries, response.status());
            },
            Err(e) => {
                warn!("Attempt {}/{}: Waiting for FastAPI server on port {}: {}",
                      attempt, max_retries, server_port, e);
            }
        }

        // Wait before next attempt
        tokio::time::sleep(retry_delay).await;
    }

    // If we reach here, we've exceeded max retries
    let mut error_msg = format!("FastAPI server failed to start on port {} after {} attempts",
                               server_port, max_retries);

    // Try to kill the process and append any output
    let _ = python_server.kill();
    append_process_output(&mut error_msg, &mut python_server);

    error!("{}\n{}\n{}", separator, error_msg, separator);
    Err(color_eyre::eyre::eyre!(error_msg))
}

/// Helper function to append process stdout/stderr to an error message
fn append_process_output(error_msg: &mut String, process: &mut std::process::Child) {
    // Try to read stdout
    if let Some(mut stdout) = process.stdout.take() {
        let mut output = String::new();
        if std::io::Read::read_to_string(&mut stdout, &mut output).is_ok() && !output.is_empty() {
            error_msg.push_str("\nStdout: ");
            error_msg.push_str(&output);
        }
    }

    // Try to read stderr
    if let Some(mut stderr) = process.stderr.take() {
        let mut output = String::new();
        if std::io::Read::read_to_string(&mut stderr, &mut output).is_ok() && !output.is_empty() {
            error_msg.push_str("\nStderr: ");
            error_msg.push_str(&output);
        }
    }
}
