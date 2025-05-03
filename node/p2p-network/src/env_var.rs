
use dotenv::dotenv;
use std::env;

pub struct EnvVars {
    pub P2P_USAGE_DB_PATH: String,
    pub PROXY_PUBLIC_KEY_PATH: String,
}

impl EnvVars {
    pub fn load() -> Self {
        dotenv().ok();
        Self {
            P2P_USAGE_DB_PATH: env::var("P2P_USAGE_DB_PATH").unwrap_or_default(),
            PROXY_PUBLIC_KEY_PATH: env::var("PROXY_PUBLIC_KEY_PATH").unwrap_or_default()
        }
    }
}