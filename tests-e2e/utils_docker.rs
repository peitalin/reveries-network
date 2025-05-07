use std::time::{Duration, Instant};
use color_eyre::{Result, eyre::anyhow};
use tracing::{info, warn, debug, error};
use std::{
    process::{Command, Stdio},
    sync::Once,
    env,
};
use reqwest;
use reqwest::Client as AsyncReqwestClient;
use tokio::time::sleep as tokio_sleep;

// Initialize logger only once across test runs
static LOGGER_INIT: Once = Once::new();

pub fn init_test_logger() {
    if LOGGER_INIT.is_completed() {
        return;
    }
    LOGGER_INIT.call_once(|| {
        env::set_var("TEST_ENV", "true");
        let _ = color_eyre::install();
        telemetry::init_logger(telemetry::LoggerConfig {
            show_log_level: true,
            show_path: true, // Shows module path, helpful for context
            ..Default::default()
        });
    });
}

/// Starts Docker services using the specified compose file and polls llm-proxy health.
pub async fn setup_docker_environment(docker_compose_file: &str) -> Result<()> {

    const LLM_PROXY_HEALTH_TIMEOUT: Duration = Duration::from_secs(10);
    const LLM_PROXY_HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(1);
    const LLM_PROXY_HEALTH_URL: &str = "https://localhost:7070/health";

    info!("Starting Docker environment using {}", docker_compose_file);
    let output = Command::new("docker-compose")
        .args(["-f", docker_compose_file, "up", "-d"])
        .output()
        .map_err(|e| anyhow!("Failed to execute docker-compose up command: {}", e))?;

    if !output.status.success() {
        error!("docker-compose up stdout: {}", String::from_utf8_lossy(&output.stdout));
        error!("docker-compose up stderr: {}", String::from_utf8_lossy(&output.stderr));
        return Err(anyhow!("docker-compose up command failed, status: {:?}", output.status.code()));
    }

    // Poll llm-proxy health check
    info!("Waiting for llm-proxy to be healthy at {}...", LLM_PROXY_HEALTH_URL);
    let health_check_start_time = Instant::now();

    let http_client = AsyncReqwestClient::builder()
        .timeout(LLM_PROXY_HEALTH_POLL_INTERVAL)
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client for llm-proxy health check: {}",e))?;

    loop {
        if health_check_start_time.elapsed() > LLM_PROXY_HEALTH_TIMEOUT {
            return Err(anyhow!("Timed out waiting for health check at {}", LLM_PROXY_HEALTH_URL));
        }

        match http_client.get(LLM_PROXY_HEALTH_URL).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    info!("llm-proxy is healthy (status: {})", response.status());
                    break;
                } else {
                    warn!("llm-proxy health check failed with status: {}. Retrying in {:?}...",
                         response.status(),
                         LLM_PROXY_HEALTH_POLL_INTERVAL
                    );
                }
            }
            Err(e) => {
                warn!(
                    "llm-proxy health check failed: {}. Retrying in {:?}...",
                    e,
                    LLM_PROXY_HEALTH_POLL_INTERVAL
                );
            }
        }
        tokio_sleep(LLM_PROXY_HEALTH_POLL_INTERVAL).await;
    }

    info!("✅ Docker environment setup complete and llm-proxy is healthy using {}.", docker_compose_file);
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
