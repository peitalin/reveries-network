#[path = "../test_utils.rs"]
mod test_utils;
#[path = "../network_utils.rs"]
mod network_utils;

use std::time::Duration;
use std::env;
use color_eyre::Result;
use jsonrpsee::core::client::ClientT;
use scopeguard::defer;
use tokio::time;
use serde_json::json;
use tracing::info;
use once_cell::sync::Lazy;
use libp2p_identity::Keypair;

// Alloy imports for signing
use alloy_primitives::{Address, B256};
use alloy_signer::Signer;
use alloy_signer_local::PrivateKeySigner;

use p2p_network::types::{
    Reverie,
    ReverieType,
    AccessCondition,
    ExecuteWithMemoryReverieResult,
    AnthropicQuery,
};
use p2p_network::create_network::{generate_peer_keys, export_libp2p_public_key};
use sha3::{Digest, Keccak256};

use self::test_utils::{
    P2PNodeCleanupGuard,
    setup_docker_environment,
    shutdown_docker_environment,
    init_test_logger,
};
use self::network_utils::start_test_network;


async fn create_signer() -> Result<PrivateKeySigner> {
    let wallet = PrivateKeySigner::random(); // Generate a random wallet
    Ok(wallet)
}

const DOCKER_COMPOSE_TEST_FILE: &str = "../docker-compose-llm-proxy-test.yml";
const TEST_NODE_PUBKEY_EXPORT_TARGET: &str = "../llm-proxy/pubkeys/p2p-node/p2p_node_test.pub.pem";
const TEST_KEY_SEED: usize = 2;

static TEST_KEYPAIR: Lazy<Keypair> = Lazy::new(|| {
    let (
        _peer_id,
        id_keys,
        _node_name,
        _umbral_key
    ) = generate_peer_keys(Some(TEST_KEY_SEED));
    id_keys
});

static TEST_SETUP: Lazy<()> = Lazy::new(|| {
    setup();
});

fn ensure_setup() {
    // p2p-node is run from root dir's perspective
    env::set_var("CA_CERT_PATH", "./llm-proxy/certs/hudsucker.cer");
    Lazy::force(&TEST_SETUP);
}

fn setup() {
    init_test_logger();

    info!("Exporting memory_test public key for seed {} to {}", TEST_KEY_SEED, TEST_NODE_PUBKEY_EXPORT_TARGET);
    if let Err(e) = export_libp2p_public_key(&TEST_KEYPAIR, TEST_NODE_PUBKEY_EXPORT_TARGET) {
        panic!("Setup failed: Could not export memory_test public key to {}: {}", TEST_NODE_PUBKEY_EXPORT_TARGET, e);
    }
    info!("✅ Exported memory_test public key.");

    if let Err(e) = setup_docker_environment(DOCKER_COMPOSE_TEST_FILE) {
        panic!("Setup failed: Could not start Docker environment: {}", e);
    }

    info!("✅ Memory test setup complete.");
}


#[tokio::test]
#[serial_test::serial]
pub async fn test_memory_reverie() -> Result<()> {
    ensure_setup();

    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;

    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|j| base_listen_port + j as u16).collect();
    let rust_node_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    dotenv::dotenv().ok();
    let _guard = P2PNodeCleanupGuard::with_ports(rust_node_ports);

    println!("Starting Rust test network...");
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;
    println!("Rust test network started successfully.");

    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let deepseek_api_key = std::env::var("DEEPSEEK_API_KEY").ok();

    let memory_secrets_and_api_keys = json!({
        "name": "Agent K",
        "anthropic_api_key": anthropic_api_key, // delegated to llm-proxy
        "deepseek_api_key": deepseek_api_key,
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

    let signer = create_signer().await?;
    let verifying_public_key: Address = signer.address();

    println!("Step 1: Spawning secret memories and delegating API keys...");
    let reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(10),
        clients[0].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                memory_secrets_and_api_keys.clone(),
                threshold,
                total_frags,
                verifying_public_key
            ]
        )
    ).await??;

    println!("Memory reverie spawned on vessel: {} {}", reverie_result.id, reverie_result.description);
    time::sleep(Duration::from_millis(5000)).await;

    println!("Step 2: Delegated API key LLM calls with memory secrets");
    let reverie_id = reverie_result.id;
    println!("Using reverie ID: {}", reverie_id);

    let digest = Keccak256::digest(reverie_id.clone().as_bytes());
    let hash = B256::from_slice(digest.as_slice());
    let signature = signer.sign_hash(&hash).await?;
    let signature_bytes = signature.as_bytes().to_vec();
    let signature_type = AccessCondition::EcdsaSignature(signature_bytes);

    println!("Executing memory reverie (this will trigger LLM calls)...");

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
        Duration::from_secs(30),
        clients[1].request::<ExecuteWithMemoryReverieResult, _>(
            "execute_with_memory_reverie",
            jsonrpsee::rpc_params![
                reverie_id,
                ReverieType::Memory,
                signature_type,
                anthropic_query
            ]
        )
    ).await?;

    println!("Memory reverie executed successfully via RPC.");
    match rpc_result {
        Err(e) => println!("Error:\n{:?}", e),
        Ok(result) => {
            println!("Claude Result: {:?}", result.claude);
            println!("Deepseek Result: {:?}", result.deepseek);
            println!("Usage Report: {:?}", result.usage_report);
        }
    };

    time::sleep(Duration::from_secs(3)).await;
    defer! { shutdown_docker_environment(DOCKER_COMPOSE_TEST_FILE); }
    Ok(())
}