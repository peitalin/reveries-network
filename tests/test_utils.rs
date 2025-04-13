use std::time::Duration;
use std::net::SocketAddr;
use color_eyre::{Result, eyre::anyhow};
use tracing::{info, warn};
use rpc::rpc_client::create_rpc_client;
use std::{
    process::{Command, Stdio},
    sync::Once,
};
use tokio::time;

// Initialize logger only once across test runs
static LOGGER_INIT: Once = Once::new();

pub fn init_test_logger() {
    LOGGER_INIT.call_once(|| {
        let _ = color_eyre::install();
        telemetry::init_logger(telemetry::LoggerConfig {
            show_log_level: true,
            show_path: true,
            ..Default::default()
        });
    });
}

pub async fn wait_for_rpc_server(port: u16) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut retries = 10;

    while retries > 0 {
        match create_rpc_client(&addr).await {
            Ok(_) => return Ok(()),
            Err(e) => {
                warn!("Waiting for RPC server on port {}: {}", port, e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
                retries -= 1;
            }
        }
    }
    Err(anyhow!(format!("RPC server failed to start on port: {}", port)))
}


pub fn kill_processes_on_ports(ports: &[u16]) {
    info!("Cleaning up processes on specified ports: {:?}", ports);

    // Get our own process ID to avoid killing ourselves
    let self_pid = std::process::id();

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
    std::thread::sleep(std::time::Duration::from_millis(1000));
}

// Define CleanupGuard once at the module level
pub struct CleanupGuard {
    ports: Vec<u16>,
}

impl CleanupGuard {
    // Constructor with custom ports
    pub fn with_ports(ports: Vec<u16>) -> Self {
        info!("Running initial cleanup with custom ports: {:?}...", ports);
        kill_processes_on_ports(&ports);
        Self {
            ports,
        }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        info!("Running guaranteed cleanup on ports: {:?}...", self.ports);
        kill_processes_on_ports(&self.ports);
    }
}
