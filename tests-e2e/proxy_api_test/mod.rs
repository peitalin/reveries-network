#[path = "../utils_docker.rs"]
mod utils_docker;

#[path = "../utils_network.rs"]
mod utils_network;

use color_eyre::{Result, eyre::anyhow};
use jsonrpsee::core::client::ClientT;
use once_cell::sync::Lazy;
use scopeguard::defer;
use tracing::{info, error};

use utils_docker::{
    init_test_logger,
    shutdown_docker_environment,
};
use utils_network::TestNodes;

/// filepath from the perspective of the root of the repo
const DOCKER_COMPOSE_TEST_FILE: &str = "./docker-compose-llm-proxy-test.yml";
// Generate a FIXED keypair using seed
const TARGET_NODE_2: usize = 2; // hardcode node2 as sender of API Key
// Lazy: once-per-module setup for proxy API tests
static SETUP_LOGGER_ONCE: Lazy<()> = Lazy::new(|| {
    init_test_logger();
    dotenv::dotenv().ok();
});


#[tokio::test]
#[serial_test::serial]
async fn test_add_and_remove_api_key_success() -> Result<()> {

    Lazy::force(&SETUP_LOGGER_ONCE);

    let test_nodes = TestNodes::new(2)
        .exec_docker_compose_with_node(
            TARGET_NODE_2,
            format!("docker-compose -f {} up -d llm-proxy", DOCKER_COMPOSE_TEST_FILE)
            // Only using llm-proxy for this test
        )
        .start_test_network().await?
        .create_rpc_clients().await?
        .wait_for_llm_proxy_key_registration(TARGET_NODE_2).await?;

    let clients = test_nodes.rpc_clients.clone();

    // Perform the add_proxy_api_key and remove_proxy_api_key calls
    let reverie_id = format!("test_reverie_{}", nanoid::nanoid!(8));
    let api_key_val = format!("test_api_key_{}", nanoid::nanoid!(12));
    let spender_address_str = "0f39d6918c19cba78586732952ead4f1ac038df3".to_string();
    let spender_type = "ecdsa".to_string();

    // EnvVars::load() inside add_proxy_api_key and remove_proxy_api_key will get LLM_PROXY_API_URL.
    // Ensure LLM_PROXY_API_URL is set in the environment where p2p-node runs.
    println!("NodeClient: Attempting to add API key to llm-proxy...");
    clients[&9902].request(
        "add_proxy_api_key",
        jsonrpsee::rpc_params![
            reverie_id.clone(),
            "ANTHROPIC_API_KEY".to_string(),
            api_key_val.clone(),
            spender_address_str.clone(),
            spender_type.clone()
        ]
    ).await.map_err(|e| anyhow!("add_proxy_api_key failed during mock test: {}", e))?;
    println!("Test: Successfully added API key to llm-proxy.");

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    println!("Test: trying to remove API key from llm-proxy...");
    clients[&9902].request(
        "remove_proxy_api_key",
        jsonrpsee::rpc_params![
            reverie_id
        ]
    ).await.map_err(|e| anyhow!("remove_proxy_api_key failed during mock test: {}", e))?;
    println!("Test: Successfully removed API key from llm-proxy.");

    let result_msg = "Mock API key test: llm-proxy started, key registered, API key added and removed successfully.".to_string();
    println!("Test: {}", result_msg);

    defer! { test_nodes.cleanup_ports(); }
    defer! { shutdown_docker_environment(3); }
    Ok(())
}
