use std::time::Duration;
use std::net::SocketAddr;
use color_eyre::{Result, eyre::anyhow, eyre};
use tracing::{info, warn};
use rpc::rpc_client::create_rpc_client;
use jsonrpsee::core::client::Client;
use std::{
    process::{Command, Stdio},
    sync::Once,
    env,
};

// Initialize logger only once across test runs
static LOGGER_INIT: Once = Once::new();

pub fn init_test_logger() {
    // This is a safe version that won't panic if called multiple times
    if LOGGER_INIT.is_completed() {
        return;
    }

    LOGGER_INIT.call_once(|| {
        // Set the environment variable for tests
        env::set_var("TEST_ENV", "true");
        let _ = color_eyre::install();
        telemetry::init_logger(telemetry::LoggerConfig {
            show_log_level: true,
            show_path: true,
            ..Default::default()
        });
    });
}

pub async fn wait_for_rpc_server(port: u16) -> Result<Client> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut retries = 30;

    while retries > 0 {
        match create_rpc_client(&addr).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                warn!("Waiting for RPC server on port {}: {}", port, e);
                tokio::time::sleep(Duration::from_millis(2000)).await;
                retries -= 1;
            }
        }
    }
    Err(anyhow!("RPC server failed to start on port: {} after retries", port))
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
pub struct P2PNodeCleanupGuard {
    ports: Vec<u16>,
}

impl P2PNodeCleanupGuard {
    // Constructor with custom ports
    pub fn with_ports(ports: Vec<u16>) -> Self {
        info!("Running initial cleanup with custom ports: {:?}...", ports);
        kill_processes_on_ports(&ports);
        Self {
            ports,
        }
    }
}

impl Drop for P2PNodeCleanupGuard {
    fn drop(&mut self) {
        info!("Running guaranteed cleanup on ports: {:?}...", self.ports);
        kill_processes_on_ports(&self.ports);
    }
}

/// Starts Docker services using the specified compose file.
pub fn setup_docker_environment(docker_compose_file: &str) -> Result<()> {
    info!("Starting Docker environment using {}...", docker_compose_file);
    // Use spawn and wait instead of output to avoid blocking, similar to memory_test
    let mut docker_compose = Command::new("docker-compose")
        .args(["-f", docker_compose_file, "up", "-d"])
        .stdout(Stdio::inherit()) // Show docker output during setup
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| anyhow!("Failed to spawn docker-compose up command: {}", e))?;

    // Wait for the command to complete
    let status = docker_compose
        .wait()
        .map_err(|e| anyhow!("Failed to wait for docker-compose up command: {}", e))?;

    if !status.success() {
        return Err(anyhow!("docker-compose up command failed, status: {:?}", status.code()));
    }

    // Allow some time for services to initialize (can be adjusted)
    // Consider adding health checks here in the future if needed.
    info!("Docker services started. Waiting a few seconds for initialization...");
    std::thread::sleep(Duration::from_secs(6));

    info!("✅ Docker environment setup complete using {}.", docker_compose_file);
    Ok(())
}

pub fn shutdown_docker_environment(docker_compose_file: &str) {
    match Command::new("docker-compose")
        .args(["-f", docker_compose_file, "down", "-v"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        // .spawn() // don't wait for docker-compose down to complete
        .output() // wait for docker-compose down to complete
    {
        Ok(_) => {
            println!("✅ 'docker-compose down -v' completed successfully.");
        }
        Err(e) => {
            println!("❌ 'docker-compose down -v' failed: {}", e);
        }
    }
}
