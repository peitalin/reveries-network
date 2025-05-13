
use dotenv::dotenv;
use std::env;
use tracing::debug;

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct EnvVars {
    pub TEST_ENV: bool,
    // p2p-node EnvVars
    pub P2P_USAGE_DB_PATH: String,
    // llm-proxy EnvVars
    pub LLM_PROXY_API_URL: String,
}

const DEFAULT_P2P_USAGE_DB_PATH: &str = "./p2p-usage.db";
// llm-proxy EnvVars
const DEFAULT_LLM_PROXY_API_URL: &str = "https://localhost:7070";


thread_local! {
    pub static NODE_SEED_NUM: std::cell::RefCell<usize> = std::cell::RefCell::new(1);
}

#[allow(non_snake_case)]
impl EnvVars {
    pub fn load() -> Self {
        dotenv().ok();
        let test_env = env::var("TEST_ENV").unwrap_or("false".to_string()).parse::<bool>().unwrap_or(false);
        Self {
            TEST_ENV: test_env,
            P2P_USAGE_DB_PATH: env::var("P2P_USAGE_DB_PATH").unwrap_or_else(|_| {
                debug!("P2P_USAGE_DB_PATH env var not set, defaulting to: {}", DEFAULT_P2P_USAGE_DB_PATH);
                DEFAULT_P2P_USAGE_DB_PATH.to_string()
            }),
            LLM_PROXY_API_URL: env::var("LLM_PROXY_API_URL").unwrap_or_else(|_| {
                debug!("LLM_PROXY_API_URL env var not set, defaulting to: {}", DEFAULT_LLM_PROXY_API_URL);
                DEFAULT_LLM_PROXY_API_URL.to_string()
            }),
        }
    }
}
