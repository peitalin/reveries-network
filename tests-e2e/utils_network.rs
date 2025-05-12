#[path = "./utils_docker.rs"]
mod utils_docker;

use std::{
    time::Duration,
    process::{Command, Stdio},
    net::SocketAddr,
    collections::HashMap,
};
use color_eyre::{eyre::eyre, Result, eyre::anyhow};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::HttpClient;
use tokio::time;
use tokio::time::timeout;
use tracing::{warn, info, debug, error};

use rpc::rpc_client::create_http_rpc_client;
use jsonrpsee::rpc_params;
use serde_json::Value;

// Node 1 is the bootstrap node
static BOOTSTRAP_PEER: &str = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
pub type Port = u16;

/// Helper function to start a network of nodes and return clients
pub async fn start_test_network(
    num_nodes: usize,
    base_rpc_port: Port,
    base_listen_port: Port
) -> Result<HashMap<Port, HttpClient>> {

    let bootstrap_url = &format!("/ip4/127.0.0.1/tcp/{}/p2p/{}", base_listen_port, BOOTSTRAP_PEER);
    // Prepare port lists
    let rpc_ports: Vec<Port> = (0..num_nodes).map(|i| base_rpc_port + i as Port).collect();
    let listen_ports: Vec<Port> = (0..num_nodes).map(|j| base_listen_port + j as Port).collect();
    if num_nodes < 5 {
        return Err(eyre!("num_nodes must be at least 5 for this test. Got {}", num_nodes));
    }

    // Start nodes (keep process handling for cleanup)
    let mut node_processes = Vec::with_capacity(num_nodes);
    for i in 0..num_nodes {
        let seed = i + 1; // Seeds start at 1
        let rpc_port = rpc_ports[i];
        let listen_port = listen_ports[i];
        let bootstrap = if i > 0 { Some(bootstrap_url) } else { None };
        if num_nodes < 2 {
            return Err(eyre!("num_nodes must be at least 2 for this test. Got {}", num_nodes));
        }

        let mut cmd = Command::new("cargo");
        cmd.current_dir("..")
            .args([
                "run", "--bin", "rpc",
                "--", // cmd args
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
                info!("Started node{} on RPC port {} and listen port {}", seed, rpc_port, listen_port);
            }
            Err(e) => {
                warn!("Failed to spawn node {}: {}", seed, e);
                // Attempt basic cleanup before returning
                for (_, mut child) in node_processes {
                    let _ = child.kill();
                }
                return Err(eyre!("Failed to spawn node {}: {}", seed, e));
            }
        }
    }

    // Give nodes time to discover each other
    println!("Waiting for peer kademlia connections to be established...");
    time::sleep(Duration::from_secs(1)).await; // Keep a very short initial pause
    println!("All nodes started and ready for client initialization.");

    // Create clients for all spawned nodes.
    let client_target_ports = rpc_ports.clone();
    let ready_when_peer_count = (num_nodes - 1).min(4);
    // Expect to connect to at least 4 nodes.

    info!("Initializing clients for ports: {:?} using TestClientsBuilder...", client_target_ports);
    info!("Expecting each client's node to have at least {} connected peers.", ready_when_peer_count);
    let clients = TestClientsBuilder::new(client_target_ports)
        .wait_for_rpc_servers().await?
        .wait_for_network_readiness(
            ready_when_peer_count,
            30, // poll_timeout_secs
            500 // poll_interval_ms previously 1000
        ).await?;

    info!("Successfully prepared {} clients through builder.", clients.len());

    Ok(clients)
}


pub fn kill_processes_on_ports(ports: &[u16]) {
    // Get our own process ID to avoid killing ourselves
    let self_pid = std::process::id();
    // First approach: Use lsof to find processes by port
    for &port in ports {
        let find_cmd = format!(
            "lsof -ti:{} | grep -v {}",
            port,
            self_pid
        );

        // Get PIDs first
        if let Ok(output) = std::process::Command::new("sh")
            .arg("-c")
            .arg(&find_cmd)
            .output()
        {
            if !output.stdout.is_empty() {
                let pids = String::from_utf8_lossy(&output.stdout);
                // Kill each PID individually
                for pid in pids.split_whitespace() {
                    debug!("Killing process {} on port {}", pid, port);
                    let _ = std::process::Command::new("kill")
                        .arg("-9")
                        .arg(pid)
                        .status();
                }
            } else {
                debug!("No processes found on port {}", port);
            }
        }
    }

    // Second approach: Use pkill to find processes by name (safer than killall)
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg("pkill -9 -f 'cargo run --bin rpc'")
        .output();

    // Give a longer delay to ensure ports are fully released
    std::thread::sleep(std::time::Duration::from_millis(1000));
}

// Define CleanupGuard once at the module level
pub struct P2PNodeCleanupGuard {
    ports: Vec<u16>,
}

impl P2PNodeCleanupGuard {
    // Constructor with custom ports
    pub fn with_ports(ports: Vec<u16>) -> Self {
        info!("Initial cleanup on ports: {:?}...", ports);
        kill_processes_on_ports(&ports);
        Self {
            ports,
        }
    }
}

impl Drop for P2PNodeCleanupGuard {
    fn drop(&mut self) {
        info!("Running final cleanup on ports: {:?}...", self.ports);
        kill_processes_on_ports(&self.ports);
    }
}

#[derive(Clone, Debug)]
pub struct TestClientsBuilder {
    target_rpc_ports: Vec<Port>,
    active_clients: HashMap<Port, HttpClient>,
}

impl TestClientsBuilder {
    pub fn new(target_ports: Vec<Port>) -> Self {
        TestClientsBuilder {
            target_rpc_ports: target_ports,
            active_clients: HashMap::new(),
        }
    }

    pub async fn wait_for_rpc_servers(mut self) -> Result<Self, color_eyre::eyre::Error> {
        info!("Attempting to connect RPC clients for ports: {:?}", self.target_rpc_ports);
        for &port in &self.target_rpc_ports {
            match self.wait_for_rpc_server(port).await {
                Ok(client) => {
                    self.active_clients.insert(port, client);
                    info!("Successfully connected RPC client for port {}", port);
                }
                Err(e) => {
                    error!("Failed to connect RPC client for port {}: {}", port, e);
                    return Err(color_eyre::eyre::anyhow!("Failed to establish RPC connection for port {}: {}", port, e));
                }
            }

        }
        Ok(self)
    }

    async fn wait_for_rpc_server(&self, port: u16) -> Result<HttpClient> {

        const RPC_READY_TIMEOUT: Duration = Duration::from_secs(20);
        const RPC_POLL_INTERVAL: Duration = Duration::from_millis(1000);

        let rpc_addr = SocketAddr::from(([127, 0, 0, 1], port));
        let start_time = std::time::Instant::now();

        loop {
            if start_time.elapsed() > RPC_READY_TIMEOUT {
                return Err(anyhow!("Timed out waiting for RPC server on {}", rpc_addr));
            }

            match timeout(RPC_POLL_INTERVAL, create_http_rpc_client(&rpc_addr)).await {
                Ok(Ok(client)) => {
                    // Attempt a simple RPC call to confirm readiness if a method like "ping" exists
                    // For now, successful client creation implies readiness for this test utility.
                    // Example if you add a "ping" method to your RPC server:
                    if client.request::<serde_json::Value, _>("health", jsonrpsee::rpc_params![]).await.is_ok() {
                    info!("RPC server connected to {}: ready.", rpc_addr);
                    return Ok(client);
                    } else {
                    info!("RPC server connected to {}: awaiting health check.", rpc_addr);
                    }
                }
                Ok(Err(e)) => {
                    info!("RPC server on {} not ready (client error: {}), retrying...", rpc_addr, e);
                }
                Err(_) => {
                    info!("RPC server on {} not ready (timed out), retrying...", rpc_addr);
                }
            }
            tokio::time::sleep(RPC_POLL_INTERVAL).await;
        }
    }

    pub async fn wait_for_network_readiness(
        self,
        ready_when_peer_count: usize,
        poll_timeout_secs: u64,
        poll_interval_ms: u64,
    ) -> Result<HashMap<Port, HttpClient>, color_eyre::eyre::Error> {
        if self.active_clients.is_empty() {
            warn!("No active RPC clients to check for network readiness. Returning empty map.");
            return Ok(HashMap::new());
        }
        println!("Checking network readiness for {} active clients...", self.active_clients.len());
        let clients_to_check: Vec<HttpClient> = self.active_clients.values().cloned().collect();

        match self.internal_wait_for_network_readiness(
            &clients_to_check,
            ready_when_peer_count,
            poll_timeout_secs,
            poll_interval_ms,
        ).await {
            Ok(_) => {
                info!("Network readiness confirmed for active clients.");
                Ok(self.active_clients)
            }
            Err(e_str) => {
                error!("Network readiness check failed: {}", e_str);
                Err(color_eyre::eyre::anyhow!("Network readiness check failed: {}", e_str))
            }
        }
    }

    /// Polls each node to check if it's connected to at least `ready_when` peers.
    async fn internal_wait_for_network_readiness(
        &self,
        clients: &[HttpClient],
        ready_when_peer_count: usize,
        poll_timeout_secs: u64,
        poll_interval_ms: u64,
    ) -> Result<(), String> {
        println!(
            "Waiting for {} active clients to connect to at least {} peers each (timeout: {}s)...",
            clients.len(),
            ready_when_peer_count,
            poll_timeout_secs
        );

        let overall_timeout = Duration::from_secs(poll_timeout_secs);
        let start_time = time::Instant::now();

        for (i, client) in clients.iter().enumerate() {

            let node_name = format!("Client {}", i);
            let mut peer_count = 0;
            println!("Checking peer readiness for {}...", node_name);

            loop {
                if start_time.elapsed() > overall_timeout {
                    return Err(format!(
                        "Timeout waiting for network readiness. {} did not report enough peers: {}/{} peers",
                        node_name,
                        peer_count,
                        ready_when_peer_count
                    ));
                }

                match client.request::<Value, _>("get_connected_peers", rpc_params![]).await {
                    Ok(response) => {
                        if let Some(peers_val) = response.get("connected_peers") {
                            if let Some(peers_array) = peers_val.as_array() {
                                peer_count = peers_array.len();
                                println!("{}: Has {} connected peers.", node_name, peer_count);
                                if peer_count >= ready_when_peer_count {
                                    println!("{}: Is ready with enough peers.", node_name);
                                    break;
                                }
                            } else {
                                eprintln!(
                                    "{}: 'connected_peers' is not an array in response: {:?}",
                                    node_name, response
                                );
                            }
                        } else {
                            eprintln!(
                                "{}: 'connected_peers' field missing in response: {:?}",
                                node_name, response
                            );
                        }
                    }
                    Err(e) => {
                        if start_time.elapsed().as_secs() % 5 == 0 || start_time.elapsed().as_secs() < 5 {
                            eprintln!(
                                "{}: Error calling get_connected_peers: {:?}. Retrying...",
                                node_name, e
                            );
                        }
                    }
                }
                time::sleep(Duration::from_millis(poll_interval_ms)).await;
            }
        }
        println!("All checked clients report their nodes are sufficiently connected.");
        println!("Elapsed time: {:?}", start_time.elapsed());
        Ok(())
    }
}
// --- End of TestClientsBuilder struct and impl ---
