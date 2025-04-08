use std::time::Duration;
use std::net::SocketAddr;
use tracing::{info, warn};
use rpc::rpc_client::create_rpc_client;
use color_eyre::Result;


pub async fn wait_for_rpc_server(port: u16) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut retries = 10;

    while retries > 0 {
        match create_rpc_client(&addr).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!("Waiting for RPC server on port {}: {}", port, e);
                tokio::time::sleep(Duration::from_millis(200)).await;
                retries -= 1;
            }
        }
    }
    Err(color_eyre::eyre::eyre!("RPC server failed to start"))
}


// Add this helper function to kill processes by their ports
pub fn kill_processes_on_ports() {
    // Call the function with the default ports list
    kill_processes_on_specific_ports(&[
        8001, 8002, 8003, 8004,
        9001, 9002, 9003, 9004,
        9101, 9201, 9202
    ])
}

// Customizable version that accepts specific ports
pub fn kill_processes_on_specific_ports(ports: &[u16]) {
    info!("Cleaning up processes on specified ports: {:?}", ports);

    // Get our own process ID to avoid killing ourselves
    let self_pid = std::process::id();
    info!("Current test process ID: {}", self_pid);

    // Kill any existing processes on these ports, using multiple approaches
    // to be thorough

    // First approach: Use lsof to find processes by port
    for &port in ports {
        let find_cmd = format!(
            "lsof -ti:{} | grep -v {}",
            port,
            self_pid
        );

        info!("Finding processes on port {}", port);

        // Get PIDs first
        if let Ok(output) = std::process::Command::new("sh")
            .arg("-c")
            .arg(&find_cmd)
            .output()
        {
            if !output.stdout.is_empty() {
                let pids = String::from_utf8_lossy(&output.stdout);
                info!("Found PIDs on port {}: {}", port, pids);

                // Kill each PID individually
                for pid in pids.split_whitespace() {
                    info!("Killing process {} on port {}", pid, port);
                    let _ = std::process::Command::new("kill")
                        .arg("-9")
                        .arg(pid)
                        .status();
                }
            } else {
                info!("No processes found on port {}", port);
            }
        }
    }

    // Second approach: Use pkill to find processes by name (safer than killall)
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg("pkill -9 -f 'cargo run --bin rpc'")
        .output();

    // Give a longer delay to ensure ports are fully released
    std::thread::sleep(std::time::Duration::from_millis(400));
}

// Define CleanupGuard once at the module level
pub struct CleanupGuard {
    ports: Vec<u16>,
}

impl CleanupGuard {
    // Constructor that runs cleanup when instantiated with default ports
    pub fn new() -> Self {
        info!("Running initial cleanup with default ports...");
        kill_processes_on_ports();
        Self {
            ports: vec![
                8001, 8002, 8003, 8004,
                9001, 9002, 9003, 9004,
                9101, 9201, 9202
            ],
        }
    }

    // Constructor with custom ports
    pub fn with_ports(ports: Vec<u16>) -> Self {
        info!("Running initial cleanup with custom ports: {:?}...", ports);
        kill_processes_on_specific_ports(&ports);
        Self {
            ports,
        }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        info!("Running guaranteed cleanup on ports: {:?}...", self.ports);
        kill_processes_on_specific_ports(&self.ports);
    }
}
