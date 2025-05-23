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
static BOOTSTRAP_PEER_PORT: Port = 9001;
static BASE_RPC_PORT: Port = 9901;
static BASE_LISTEN_PORT: Port = 9001;
pub type Port = u16;


#[derive(Clone, Debug)]
pub struct TestNodeConfig {
    pub rpc_port: Port,
    pub listen_port: Port,
    pub seed: usize,
    pub bootstrap_peer: Option<String>,
    pub docker_compose_cmd: Option<String>,
}

impl TestNodeConfig {
    pub fn new(
        rpc_port: Port,
        listen_port: Port,
        seed: usize,
        bootstrap_peer: Option<String>,
        docker_compose_cmd: Option<String>
    ) -> Self {
        Self {
            rpc_port,
            listen_port,
            seed,
            bootstrap_peer,
            docker_compose_cmd
        }
    }
}

#[derive(Clone, Debug)]
pub struct TestNodes {
    pub node_configs: Vec<TestNodeConfig>,
    pub rpc_ports: Vec<Port>,
    pub listen_ports: Vec<Port>,
    pub rpc_clients: HashMap<Port, HttpClient>,
}

impl TestNodes {
    pub fn exec_docker_compose_with_node(mut self, node_number: usize, docker_compose_cmd: String) -> Self {
        let index = node_number - 1;
        self.node_configs[index].docker_compose_cmd = Some(docker_compose_cmd.clone());
        self
    }
}

impl TestNodes {
    pub fn new(num_nodes: usize) -> TestNodes {
        let bootstrap_url = format!("/ip4/127.0.0.1/tcp/{}/p2p/{}", BOOTSTRAP_PEER_PORT, BOOTSTRAP_PEER);
        let rpc_ports: Vec<Port> = (0..num_nodes).map(|i| BASE_RPC_PORT + i as Port).collect();
        let listen_ports: Vec<Port> = (0..num_nodes).map(|j| BASE_LISTEN_PORT + j as Port).collect();
        let all_ports = [&rpc_ports[..], &listen_ports[..]].concat();
        let mut test_node_configs = Vec::with_capacity(num_nodes);

        for i in 0..num_nodes {
            let seed = i + 1; // Seeds start at 1
            let rpc_port = rpc_ports[i];
            let listen_port = listen_ports[i];
            let bootstrap = if i > 0 { Some(bootstrap_url.clone()) } else { None };
            test_node_configs.push(TestNodeConfig::new(
                rpc_port,
                listen_port,
                seed,
                bootstrap,
                None
            ));
        }

        info!("Initial cleanup on ports: {:?}...", all_ports);
        kill_processes_on_ports(&all_ports);

        info!("Starting Rust test network...");
        TestNodes {
            node_configs: test_node_configs,
            rpc_ports: rpc_ports,
            listen_ports: listen_ports,
            rpc_clients: HashMap::new(),
        }
    }

    pub fn required_nodes(self, required_num_nodes: usize) -> Result<Self> {
        let num_nodes = self.node_configs.len();
        if self.node_configs.len() < required_num_nodes {
            Err(anyhow!("Test requires {} nodes, only {} provided", required_num_nodes, num_nodes))
        } else {
            Ok(self)
        }
    }

    pub fn all_ports(&self) -> Vec<Port> {
        [&self.rpc_ports[..], &self.listen_ports[..]].concat()
    }

    pub async fn start_test_network(self) -> Result<Self> {

        let test_nodes = &self.node_configs;
        let mut node_processes = Vec::with_capacity(test_nodes.len());

        for node in test_nodes.clone() {
            let mut cmd = Command::new("cargo");
            cmd.current_dir("..")
                .args([
                    "run", "--bin", "rpc",
                    "--", // cmd args
                    "--secret-key-seed", &node.seed.to_string(),
                    "--rpc-port", &node.rpc_port.to_string(),
                    "--listen-address", &format!("/ip4/0.0.0.0/tcp/{}", node.listen_port),
                ])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            if let Some(bootstrap_peer) = node.bootstrap_peer {
                cmd.args(["--bootstrap-peers", &bootstrap_peer]);
            }

            if let Some(docker_compose_cmd) = node.docker_compose_cmd {
                cmd.args(["--docker-compose-cmd", &docker_compose_cmd]);
            }

            match cmd.spawn() {
                Ok(child) => {
                    node_processes.push((node.rpc_port, child));
                    info!("Started node{} on RPC port {} and listen port {}", node.seed, node.rpc_port, node.listen_port);
                }
                Err(e) => {
                    warn!("Failed to spawn node {}: {}", node.seed, e);
                    // Attempt basic cleanup before returning
                    for (_, mut child) in node_processes {
                        let _ = child.kill();
                    }
                    return Err(eyre!("Failed to spawn node {}: {}", node.seed, e));
                }
            }
        }

        // Give nodes time to discover each other
        println!("Waiting for peer kademlia connections to be established...");
        time::sleep(Duration::from_secs(1)).await; // Keep a very short initial pause
        println!("All nodes started and ready for client initialization.");
        Ok(self)
    }

    pub async fn create_rpc_clients(mut self) -> Result<Self> {
        // ensure all nodes are connected to each other
        let ready_when_peer_count = &self.node_configs.len() - 1;
        println!("Initializing clients for ports: {:?}", &self.rpc_ports);
        println!("Expecting each client's node to have at least {} connected peers.", ready_when_peer_count);
        self.rpc_clients = self.clone()
            .wait_for_rpc_servers().await?
            .wait_for_network_readiness(
                ready_when_peer_count,
                20, // poll_timeout_secs
                500 // poll_interval_ms previously 1000
            ).await?;

        println!("Successfully prepared {} clients through builder.", self.rpc_clients.len());
        Ok(self)
    }

    async fn wait_for_rpc_servers(mut self) -> Result<Self> {
        info!("Attempting to connect RPC clients for ports: {:?}", self.rpc_ports);
        for &port in &self.rpc_ports {
            match self.internal_wait_for_rpc_server(port).await {
                Ok(client) => {
                    self.rpc_clients.insert(port, client);
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

    async fn internal_wait_for_rpc_server(&self, port: u16) -> Result<HttpClient> {
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
    ) -> Result<HashMap<Port, HttpClient>> {
        if self.rpc_clients.is_empty() {
            warn!("No active RPC clients to check for network readiness. Returning empty map.");
            return Ok(HashMap::new());
        }
        println!("Checking network readiness for {} active clients...", self.rpc_clients.len());
        let clients_to_check: Vec<HttpClient> = self.rpc_clients.values().cloned().collect();

        match self.internal_wait_for_network_readiness(
            &clients_to_check,
            ready_when_peer_count,
            poll_timeout_secs,
            poll_interval_ms,
        ).await {
            Ok(_) => {
                info!("Network readiness confirmed for active clients.");
                Ok(self.rpc_clients.clone())
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
    ) -> Result<()> {
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
                    return Err(anyhow!(
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
                                println!("{}: connected to {}/{} peers", node_name, peer_count, ready_when_peer_count);
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

    pub fn cleanup_ports(&self) {
        info!("Running final cleanup on ports: {:?}...", &self.all_ports());
        kill_processes_on_ports(&self.all_ports());
    }

    pub async fn wait_for_llm_proxy_key_registration(self, node_number: usize) -> Result<Self> {

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        // TODO: order matters, need to create rpc clients first so refactor Builder to
        // account for this
        let port = self.rpc_ports[node_number - 1];
        if self.rpc_clients.get(&port).is_none() {
            return Err(anyhow!("RPC client missing on port {}, call create_rpc_clients() first", port));
        }

        println!("NodeClient: Waiting for llm-proxy to register its P256 public key...");
        let wait_duration = std::time::Duration::from_secs(20);
        let poll_interval = std::time::Duration::from_millis(500);
        let start_wait = tokio::time::Instant::now();

        loop {
            let (
                proxy_public_key,
                proxy_ca_cert
            ): (Option<String>, Option<String>) = self.rpc_clients[&port].request(
                "get_proxy_public_key",
                jsonrpsee::rpc_params![]
            ).await?;

            if proxy_public_key.is_some() && proxy_ca_cert.is_some() {
                println!("NodeClient: llm-proxy's public key and CA certificate have been registered.");
                break;
            }
            println!("still waiting for proxy_public_key: {:?}", proxy_public_key);
            println!("still waiting for proxy_ca_cert: {:?}", proxy_ca_cert);
            if start_wait.elapsed() > wait_duration {
                error!("NodeClient: Timeout waiting for llm-proxy to register its public key.");
                let _ = std::process::Command::new("docker").args(["logs", "llm-proxy"]).status();
                return Err(anyhow!("Timeout waiting for llm-proxy key registration"));
            }
            tokio::time::sleep(poll_interval).await;
        }
        Ok(self)
    }
}


fn kill_processes_on_ports(ports: &[u16]) {
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

    // delay to ensure ports are fully released
    // std::thread::sleep(std::time::Duration::from_millis(500));
}
