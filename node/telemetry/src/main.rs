mod metrics;
use metrics::{REQUESTS_TOTAL, init_telemetry};

#[tokio::main]
pub async fn main() -> color_eyre::Result<()> {

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
