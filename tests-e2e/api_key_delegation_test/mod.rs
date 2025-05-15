#[path = "../utils_docker.rs"]
mod utils_docker;
#[path = "../utils_network.rs"]
mod utils_network;

use alloy_primitives::{Address, B256};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use color_eyre::{Result, eyre::anyhow};
use jsonrpsee::core::client::ClientT;
use std::time::Duration;
use scopeguard::defer;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tokio::time;
use tracing::{info, error};
use tokio::sync::OnceCell;

// internal imports
use p2p_network::types::{
    Reverie,
    ReverieType,
    AccessKey,
    ExecuteWithMemoryReverieResult,
    AnthropicQuery,
};
use utils_docker::{
    shutdown_docker_environment,
    init_test_logger,
};
use utils_network::{
    TestNodes,
};

/// filepath from the perspective of the root of the repo
const DOCKER_COMPOSE_TEST_FILE: &str = "./docker-compose-llm-proxy-test.yml";
// Generate a FIXED keypair using seed
const TARGET_NODE_2: usize = 2; // hardcode node2 as sender of API Key

static TEST_ENV_SETUP_ONCE: OnceCell<()> = OnceCell::const_new();

async fn setup_test_environment_once_async() -> Result<()> {
    TEST_ENV_SETUP_ONCE.get_or_try_init(|| async {
        init_test_logger();
        Ok::<(), color_eyre::eyre::Error>(())
    }).await?;
    Ok(())
}

async fn create_signer() -> Result<PrivateKeySigner> {
    let wallet = PrivateKeySigner::random(); // Generate a random wallet
    Ok(wallet)
}


#[tokio::test]
#[serial_test::serial]
pub async fn test_api_key_delegation_reverie() -> Result<()> {

    setup_test_environment_once_async().await?;

    let test_nodes = TestNodes::new(5)
        .exec_docker_compose_with_node(
            TARGET_NODE_2,
            format!("docker-compose -f {} up -d",  DOCKER_COMPOSE_TEST_FILE)
        )
        .start_test_network().await?
        .create_rpc_clients().await?;

    let clients = test_nodes.rpc_clients.clone();

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // Wait for llm-proxy to register its pubkey with this p2p-node
    println!("NodeClient: Waiting for llm-proxy to register its P256 public key...");
    let wait_duration = std::time::Duration::from_secs(20);
    let poll_interval = std::time::Duration::from_millis(500);
    let start_wait = tokio::time::Instant::now();
    loop {
        let (
            proxy_public_key,
            proxy_ca_cert
        ): (Option<String>, Option<String>) = clients[&9902].request(
            "get_proxy_public_key",
            jsonrpsee::rpc_params![]
        ).await?;

        if proxy_public_key.is_some() && proxy_ca_cert.is_some() {
            println!("NodeClient: LLM Proxy's P256 public key and CA certificate have been registered.");
            break;
        }
        println!("still waiting for proxy_public_key: {:?}", proxy_public_key);
        println!("still waiting for proxy_ca_cert: {:?}", proxy_ca_cert);
        if start_wait.elapsed() > wait_duration {
            error!("NodeClient: Timeout waiting for llm-proxy to register its public key.");
            let _ = std::process::Command::new("docker").args(["logs", "llm-proxy"]).status();
            return Err(anyhow!("Timeout waiting for llm-proxy key registration"));
        }
        tokio::time::sleep(poll_interval).await;
    }
    println!("NodeClient: Confirmed LLM Proxy P256 public key and CA certificate are present in NodeClient state.");

    // Encrypt Secret #1: Anthropic API key
    let api_keys_secrets = json!({
        "anthropic_api_key": std::env::var("ANTHROPIC_API_KEY").ok(),
        "deepseek_api_key": std::env::var("DEEPSEEK_API_KEY").ok(),
    });

    // Encrypt Secret #2: Memories
    let memory_secrets = json!({
        "name": "Agent K",
        "memories": [
            {
                "name": "First Beach Trip",
                "memory": "I remember the first time I saw the ocean. The waves were gentle, \
                    the sand was warm between my toes, and the salty breeze carried \
                    the sound of seagulls. It was a perfect summer day.",
                "location": {
                    "name": "Miami",
                    "state": "FL",
                }
            },
            {
                "name": "Mountain Storm",
                "memory": "The clouds swirled around the mountain peaks, dark and heavy with rain. \
                    Lightning flashed, illuminating the valley below in brief, electric moments. \
                    I stood beneath the ancient pine, feeling the rumble of thunder in my chest \
                    as the first fat droplets began to fall. There was something cleansing in \
                    watching the storm from that vantage point—like witnessing the world being remade.",
                "location": {
                    "name": "Denver",
                    "state": "CO",
                }
            },
            {
                "name": "Desert Sunrise",
                "memory": "The sky turned from deep indigo to fiery orange as the sun crested \
                    the distant mesas. The cold night air quickly gave way to the day's heat, \
                    and I watched as the desert came alive around me. Lizards emerged to bask \
                    on sun-warmed rocks, and the scent of sage filled the morning air.",
                "location": {
                    "name": "Phoenix",
                    "state": "AZ",
                }
            },
            {
                "name": "LA Smog Sunset",
                "memory": "Sitting on the Griffith Observatory lawn, watching the sun dip \
                    below the hazy skyline. The city lights twinkled on, a vast electric tapestry \
                    stretching to the sea. The air tasted thick, a mix of exhaust and possibility.",
                "location": {
                    "name": "Los Angeles",
                    "state": "CA"
                }
            }
        ],
    });

    let threshold = 2;
    let total_frags = 3;

    // Dev's signer and public key
    let signer_dev = create_signer().await?;
    let access_key_dev: Address = signer_dev.address();
    // User's signer and public key
    let signer_user = create_signer().await?;
    let access_key_user: Address = signer_user.address();

    println!("Step 1: Spawn API Key Reverie with encrypted Anthropic API key...");
    let api_key_reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&9901].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                api_keys_secrets.clone(),
                threshold,
                total_frags,
                access_key_dev
            ]
        )
    ).await??;

    println!("API Key reverie spawned on vessel: {} {}", api_key_reverie_result.id, api_key_reverie_result.description);
    time::sleep(Duration::from_millis(2000)).await;

    println!("Step 2: Delegate Anthropic API key to dev's server...");
    let signature_type_dev = {
        let digest = Keccak256::digest(api_key_reverie_result.id.clone().as_bytes());
        let hash = B256::from_slice(digest.as_slice());
        let signature = signer_dev.sign_hash(&hash).await?;
        AccessKey::EcdsaSignature(signature.as_bytes().to_vec())
    };

    clients[&9902].request(
        "delegate_api_key",
        jsonrpsee::rpc_params![
            api_key_reverie_result.id,
            ReverieType::Memory,
            signature_type_dev
        ]
    ).await?;

    println!("Step 3: Delegate secret context/memories...");
    let memory_reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&9901].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                memory_secrets.clone(),
                threshold,
                total_frags,
                access_key_user
            ]
        )
    ).await??;

    println!("Memory reverie spawned on vessel: {} {}", memory_reverie_result.id, memory_reverie_result.description);
    time::sleep(Duration::from_millis(1000)).await;

    println!("Step 4: Execute LLM calls with secret memory and delegated API Key");

    let signature_type_user = {
        let digest = Keccak256::digest(memory_reverie_result.id.clone().as_bytes());
        let hash = B256::from_slice(digest.as_slice());
        let signature = signer_user.sign_hash(&hash).await?;
        AccessKey::EcdsaSignature(signature.as_bytes().to_vec())
    };

    let anthropic_query = AnthropicQuery {
        prompt: String::from("
            Please follow these instructions:
            1. Select ONE random memory from my memories. (Use Los Angeles).
            2. Check the current weather forecast for the location associated with that memory (using the latitude and longitude coordinates).
            3. Create a poem that blends my original memory with the current weather conditions at that location.
            4. Begin your response by stating which memory you chose and the weather you found.
        "),
        tools: Some(json!([
            {
                "name": "get_forecast",
                "description": "Get weather forecast for a location based on latitude and longitude.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "latitude": {
                            "type": "number",
                            "description": "The latitude of the location"
                        },
                        "longitude": {
                            "type": "number",
                            "description": "The longitude of the location"
                        }
                    },
                    "required": ["latitude", "longitude"]
                }
            }
        ])),
        stream: Some(false)
    };

    let rpc_result = tokio::time::timeout(
        Duration::from_secs(20), // Increased timeout for LLM tool-use calls
        clients[&9902].request::<ExecuteWithMemoryReverieResult, _>(
            "execute_with_memory_reverie",
            jsonrpsee::rpc_params![
                memory_reverie_result.id,
                ReverieType::Memory,
                signature_type_user,
                anthropic_query
            ]
        )
    ).await?;

    println!("Memory reverie executed successfully via RPC.");
    match rpc_result {
        Err(e) => println!("Error:\n{:?}", e),
        Ok(result) => {
            println!("\nClaude Result: {:?}", result.claude);
            println!("Deepseek Result: {:?}", result.deepseek);
            println!("Usage Report: {:?}\n", result.usage_report);
        }
    };

    time::sleep(Duration::from_secs(1)).await;
    defer! { test_nodes.cleanup_ports(); }
    defer! { shutdown_docker_environment(3); }
    Ok(())
}