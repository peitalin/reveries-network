// llm-proxy/src/config.rs
use std::env;
use tracing::info;

#[derive(Debug, Clone)]
pub struct EnvVars {
    pub report_usage_url: String,
    pub db_path: Option<String>, // Option to easily represent memory vs file
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

        // Load LLM_PROXY_DB_PATH
        let db_path = env::var("LLM_PROXY_DB_PATH").ok();
        if let Some(ref path) = db_path {
            info!("LLM_PROXY_DB_PATH is set, using file-based DB: {}", path);
        } else {
            info!("LLM_PROXY_DB_PATH not set, using in-memory DB.");
        }

        EnvVars {
            report_usage_url,
            db_path,
        }
    }
}