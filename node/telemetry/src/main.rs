mod metrics;

use color_eyre::Result;
use metrics::REQUESTS_TOTAL;
use prometheus_exporter;

#[tokio::main]
pub async fn main() -> Result<()> {

    color_eyre::install()?;
    let url = init_telemetry()?;
    println!("Running Prometheus on: {}", url);

    // TODO: replace with application level telemetry
    // Simulate application logic - increment counter
    loop {
        REQUESTS_TOTAL.inc();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Ok(())
}

/// Starts the metrics server on port 9200
pub fn init_telemetry() -> Result<String> {
    dotenv::dotenv().ok();
    let port = std::env::var("PROMETHEUS_PORT")
        .unwrap_or("9090".to_string())
        .parse().unwrap_or("9090".to_string());

    let url = format!("0.0.0.0:{}", port);
    prometheus_exporter::start(url.parse()?)?;
    Ok(url)
}