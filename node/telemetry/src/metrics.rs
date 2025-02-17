use lazy_static::lazy_static;
use prometheus_exporter::prometheus::{
    register_int_gauge,
    register_int_counter,
    IntGauge,
    IntCounter
};

lazy_static! {

    pub static ref REQUESTS_TOTAL: IntCounter = register_int_counter!(
        "requests_total",
        "Total number of requests"
    ).unwrap();

     /// Tracks the block number of the most recent finalized head.
    pub static ref FINALIZED_HEAD: IntGauge =
        register_int_gauge!("finalized_head", "finalized head number").unwrap();

    /// Tracks the block number considered to be the safe head.
    pub static ref SAFE_HEAD: IntGauge =
        register_int_gauge!("safe_head", "safe head number").unwrap();

    /// Monitors if the node is fully synced
    pub static ref SYNCED: IntGauge = register_int_gauge!("synced", "synced flag").unwrap();
}

/// Starts the metrics server on port 9200
pub fn init_telemetry() -> color_eyre::Result<String> {
    dotenv::dotenv().ok();
    let port = std::env::var("PROMETHEUS_PORT")
        .unwrap_or("9090".to_string())
        .parse().unwrap_or("9090".to_string());

    let url = format!("0.0.0.0:{}", port);
    prometheus_exporter::start(url.parse()?)?;
    Ok(url)
}
