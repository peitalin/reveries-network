// llm-proxy/src/config.rs
use std::env;
use tracing::info;

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct EnvVars {
    pub REPORT_USAGE_URL: String,
    pub INTERNAL_API_HOST: String, // New field for host
    pub INTERNAL_API_PORT: u16,
}

#[allow(non_snake_case)]
impl EnvVars {
    /// Loads configuration from environment variables.
    pub fn load() -> Self {
        // Load REPORT_USAGE_URL with a default
        let REPORT_USAGE_URL = env::var("REPORT_USAGE_URL")
            .unwrap_or_else(|_| {
                info!("REPORT_USAGE_URL not set, using default: http://localhost:9902/report_usage");
                "http://localhost:9902/report_usage".to_string()
            });

        let INTERNAL_API_HOST = env::var("LLM_PROXY_INTERNAL_API_HOST")
            .unwrap_or_else(|_| {
                info!("LLM_PROXY_INTERNAL_API_HOST not set, using default: localhost");
                "localhost".to_string()
            });

        // Load LLM_PROXY_INTERNAL_PORT
        let INTERNAL_API_PORT = env::var("LLM_PROXY_INTERNAL_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or_else(|| {
                info!("LLM_PROXY_INTERNAL_PORT not set or invalid, using default: 7070");
                7070
            });

        EnvVars {
            REPORT_USAGE_URL,
            INTERNAL_API_HOST,
            INTERNAL_API_PORT,
        }
    }
}