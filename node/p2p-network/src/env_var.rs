
use dotenv::dotenv;
use std::env;
use tracing::debug;

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct EnvVars {
    pub TEST_ENV: bool,
    // p2p-node EnvVars
    pub P2P_USAGE_DB_PATH: String,
    pub P2P_NODE_PUBKEY_PATH: String,
    // llm-proxy EnvVars
    pub PROXY_PUBLIC_KEY_PATH: String,
    pub CA_CERT_PATH: String,
    pub PROXY_API_BASE_URL: String,
}

// p2p-node EnvVars
const P2P_NODE_PUBKEY_PATH_TEST: &str = "./llm-proxy/pubkeys/p2p-node/p2p_node_test.pub.pem";
const P2P_NODE_PUBKEY_PATH_PROD: &str = "./llm-proxy/pubkeys/p2p-node/p2p_node.pub.pem";
const DEFAULT_P2P_USAGE_DB_PATH: &str = "./p2p-usage.db";
// llm-proxy EnvVars
const DEFAULT_PROXY_PUBLIC_KEY_PATH: &str = "./llm-proxy/pubkeys/llm-proxy/llm_proxy.pub.pem";
const DEFAULT_CA_CERT_PATH_FOR_TESTS: &str = "./llm-proxy/certs/hudsucker.cer";
const DEFAULT_PROXY_API_BASE_URL: &str = "https://localhost:7070";


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
            P2P_NODE_PUBKEY_PATH: if test_env {
                P2P_NODE_PUBKEY_PATH_TEST.to_string()
            } else {
                P2P_NODE_PUBKEY_PATH_PROD.to_string()
            },
            P2P_USAGE_DB_PATH: env::var("P2P_USAGE_DB_PATH").unwrap_or_else(|_| {
                debug!("P2P_USAGE_DB_PATH env var not set, defaulting to: {}", DEFAULT_P2P_USAGE_DB_PATH);
                DEFAULT_P2P_USAGE_DB_PATH.to_string()
            }),
            PROXY_PUBLIC_KEY_PATH: env::var("PROXY_PUBLIC_KEY_PATH").unwrap_or_else(|_| {
                debug!("PROXY_PUBLIC_KEY_PATH env var not set, defaulting to: {}", DEFAULT_PROXY_PUBLIC_KEY_PATH);
                DEFAULT_PROXY_PUBLIC_KEY_PATH.to_string()
            }),
            CA_CERT_PATH: env::var("CA_CERT_PATH").unwrap_or_else(|_| {
                debug!("CA_CERT_PATH env var not set, defaulting to test path: {}", DEFAULT_CA_CERT_PATH_FOR_TESTS);
                DEFAULT_CA_CERT_PATH_FOR_TESTS.to_string()
            }),
            PROXY_API_BASE_URL: env::var("PROXY_API_BASE_URL").unwrap_or_else(|_| {
                debug!("PROXY_API_BASE_URL env var not set, defaulting to: {}", DEFAULT_PROXY_API_BASE_URL);
                DEFAULT_PROXY_API_BASE_URL.to_string()
            }),
        }
    }
}
