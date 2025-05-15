use std::env;
use tracing::info;

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct EnvVars {
    pub REPORT_USAGE_URL: String,
    pub INTERNAL_API_KEY_SERVER_PORT: u16,
    pub HUDSUCKER_PROXY_PORT: u16,
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

        // Load INTERNAL_API_KEY_SERVER_PORT
        let INTERNAL_API_KEY_SERVER_PORT= env::var("INTERNAL_API_KEY_SERVER_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or_else(|| {
                info!("INTERNAL_API_KEY_SERVER_PORT not set or invalid, using default: 7070");
                7070
            });

        let HUDSUCKER_PROXY_PORT = env::var("HUDSUCKER_PROXY_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or_else(|| {
                info!("HUDSUCKER_PROXY_PORT not set or invalid, using default: 7666");
                7666
            });

        EnvVars {
            REPORT_USAGE_URL,
            INTERNAL_API_KEY_SERVER_PORT,
            HUDSUCKER_PROXY_PORT,
        }
    }
}