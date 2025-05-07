#[path = "../utils_docker.rs"]
mod utils_docker;
#[path = "../utils_network.rs"]
mod utils_network;

use alloy_primitives::{Address, B256};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;
use color_eyre::Result;
use jsonrpsee::core::client::ClientT;
use libp2p_identity::Keypair;
use once_cell::sync::Lazy;
use std::time::Duration;
use std::env;
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
use p2p_network::utils::pubkeys::{
    generate_peer_keys,
    export_libp2p_public_key
};
use utils_docker::{
    setup_docker_environment,
    shutdown_docker_environment,
    init_test_logger,
};
use utils_network::{
    P2PNodeCleanupGuard,
    start_test_network,
};

// Path to /tests dir
const DOCKER_COMPOSE_TEST_FILE: &str = "../docker-compose-llm-proxy-test.yml";
const TEST_NODE_PUBKEY_EXPORT_TARGET: &str = "../llm-proxy/pubkeys/p2p-node/p2p_node_test.pub.pem";
// Generate a FIXED keypair using seed
const TEST_KEY_SEED: usize = 2; // hardcode node2 as sender of API Key
static TEST_KEYPAIR: Lazy<Keypair> = Lazy::new(|| {
    let (
        _peer_id,
        id_keys,
        _node_name,
        _umbral_key
    ) = generate_peer_keys(Some(TEST_KEY_SEED));
    id_keys
});

static TEST_ENV_SETUP_ONCE: OnceCell<()> = OnceCell::const_new();

async fn setup_test_environment_once_async() -> Result<()> {
    TEST_ENV_SETUP_ONCE.get_or_try_init(|| async {

        init_test_logger();
        env::set_var("CA_CERT_PATH", "./llm-proxy/certs/hudsucker.cer");

        if let Err(e) = export_libp2p_public_key(&TEST_KEYPAIR, TEST_NODE_PUBKEY_EXPORT_TARGET) {
            error!("Setup panic: Could not export memory_test public key to {}: {}", TEST_NODE_PUBKEY_EXPORT_TARGET, e);
            return Err(e);
        }
        info!("✅ Exported memory_test public key.");

        if let Err(e) = setup_docker_environment(DOCKER_COMPOSE_TEST_FILE).await {
            error!("Setup panic: Could not start Docker environment: {}", e);
            return Err(e);
        }
        Ok(())
    }).await?;
    Ok(())
}

async fn create_signer() -> Result<PrivateKeySigner> {
    let wallet = PrivateKeySigner::random(); // Generate a random wallet
    Ok(wallet)
}


#[tokio::test]
#[serial_test::serial]
pub async fn test_memory_reverie() -> Result<()> {

    setup_test_environment_once_async().await?;
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;

    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|j| base_listen_port + j as u16).collect();
    let rust_node_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    dotenv::dotenv().ok();
    let _guard = P2PNodeCleanupGuard::with_ports(rust_node_ports);

    info!("Starting Rust test network...");
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;
    info!("Rust test network started successfully.");

    // Secret #1: Anthropic API key
    let api_keys_secrets = json!({
        "anthropic_api_key": std::env::var("ANTHROPIC_API_KEY").ok(),
        "deepseek_api_key": std::env::var("DEEPSEEK_API_KEY").ok(),
    });

    // Secret #2: Memories
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

    println!("Step 1: Spawn Reverie with encrypted Anthropic API key...");
    let api_key_reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&8001].request(
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

    clients[&8002].request(
        "delegate_api_key",
        jsonrpsee::rpc_params![
            api_key_reverie_result.id,
            ReverieType::Memory,
            signature_type_dev
        ]
    ).await?;

    println!("Step 3: Spawning secret memories...");
    let memory_reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&8001].request(
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
        Duration::from_secs(20), // Increased timeout to 20 seconds for LLM tool-use calls
        clients[&8002].request::<ExecuteWithMemoryReverieResult, _>(
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

    time::sleep(Duration::from_secs(3)).await;
    defer! { shutdown_docker_environment(DOCKER_COMPOSE_TEST_FILE); }
    Ok(())
}