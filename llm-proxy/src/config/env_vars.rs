// llm-proxy/src/config.rs
use std::env;
use tracing::info;

const DEFAULT_INTERNAL_API_PORT: u16 = 7070;

#[derive(Debug, Clone)]
pub struct EnvVars {
    pub report_usage_url: String,
    pub internal_api_port: u16, // Add port field
}

impl EnvVars {
    /// Loads configuration from environment variables.
    pub fn load() -> Self {
        // Load REPORT_USAGE_URL with a default
        let report_usage_url = env::var("REPORT_USAGE_URL")
            .unwrap_or_else(|_| {
                info!("REPORT_USAGE_URL not set, using default: http://localhost:8002/report_usage");
                "http://localhost:8002/report_usage".to_string()
            });

        // Load LLM_PROXY_INTERNAL_PORT
        let internal_api_port = env::var("LLM_PROXY_INTERNAL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or_else(|| {
                info!("LLM_PROXY_INTERNAL_PORT not set or invalid, using default: {}", DEFAULT_INTERNAL_API_PORT);
                DEFAULT_INTERNAL_API_PORT
            });

        EnvVars {
            report_usage_url,
            internal_api_port, // Initialize field
        }
    }
}