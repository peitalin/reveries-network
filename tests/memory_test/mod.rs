#[path = "../test_utils.rs"]
mod test_utils;
#[path = "../network_utils.rs"]
mod network_utils;

use std::process::Command;
use std::time::Duration;
use color_eyre::{eyre::eyre, Result};
use jsonrpsee::core::client::ClientT;
use scopeguard::defer;
use tokio::time;
use serde_json::json;
use tracing::{info, warn};

use p2p_network::types::{
    Reverie,
    ReverieType,
    SignatureType,
};
use ethers::{
    core::utils::Anvil,
    signers::{LocalWallet, Signer},
};
use sha3::{Digest, Keccak256};

use self::test_utils::CleanupGuard;
use self::network_utils::start_test_network;


async fn create_signer() -> Result<LocalWallet> {
    let anvil = Anvil::new().spawn();
    let wallet: LocalWallet = anvil.keys()[0].clone().into();
    Ok(wallet)
}


static DOCKER_COMPOSE_FILE: &'static str = "../docker-compose-llm-proxy.yml";

#[tokio::test]
#[serial_test::serial]
pub async fn test_memory_reverie() -> Result<()> {
    // Define the ports for the Rust nodes
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;
    // Note: Python server port 8000 is now managed by Docker

    // Create port lists for the CleanupGuard (Rust nodes only)
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|j| base_listen_port + j as u16).collect();
    let rust_node_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up Rust node ports
    let _guard = CleanupGuard::with_ports(rust_node_ports);

    // Start Docker services (Python LLM server and LLM proxy)
    println!("Starting Docker services (this may take a while)...");

    // Use spawn and wait instead of output to avoid blocking
    let mut docker_compose = Command::new("docker-compose")
        .args(["-f", DOCKER_COMPOSE_FILE, "up", "-d"])
        .spawn()?;

    // Wait for a maximum of 2 minutes for Docker Compose to start
    let status = docker_compose.wait()?;

    if !status.success() {
        return Err(eyre!(
            "Failed to start Docker services, exit code: {:?}",
            status.code()
        ));
    }
    println!("Docker services started successfully.");

    // Schedule Docker services cleanup using scopeguard
    defer! {
        println!("Stopping LLM Docker services...");
        let docker_compose_down = Command::new("docker-compose")
            .args(["-f", DOCKER_COMPOSE_FILE, "down", "-v"])
            .output();

        match docker_compose_down {
            Ok(output) if output.status.success() => {
                println!("LLM Docker services stopped successfully.");
            }
            Ok(output) => {
                warn!(
                    "Failed to stop Docker services cleanly: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            Err(e) => {
                warn!("Error executing docker-compose down: {}", e);
            }
        }
    }

    // Allow some time for Docker services to initialize fully
    time::sleep(Duration::from_secs(4)).await;

    // Start test network (Rust nodes) and get client connections
    println!("Starting Rust test network...");
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;
    println!("Rust test network started successfully.");

    dotenv::dotenv().ok();
    let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
    let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
    let deepseek_api_key = std::env::var("DEEPSEEK_API_KEY").ok();

    // Create a test memory payload
    let memory_secrets = json!({
        "name": "Agent K",
        "memories": [
            {
                "name": "First Beach Trip",
                "memory": "I remember the first time I saw the ocean. The waves were gentle, \
                    the sand was warm between my toes, and the salty breeze carried \
                    the sound of seagulls. It was a perfect summer day.",
                "location": {
                    "name": "Miami Beach",
                    "state": "FL",
                    "lat": 25.7907,
                    "lon": -80.1300,
                    "type": "coastal"
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
                    "name": "Rocky Mountain National Park",
                    "state": "CO",
                    "lat": 40.3428,
                    "lon": -105.6836,
                    "type": "mountainous"
                }
            },
            {
                "name": "Desert Sunrise",
                "memory": "The sky turned from deep indigo to fiery orange as the sun crested \
                    the distant mesas. The cold night air quickly gave way to the day's heat, \
                    and I watched as the desert came alive around me. Lizards emerged to bask \
                    on sun-warmed rocks, and the scent of sage filled the morning air.",
                "location": {
                    "name": "Sedona",
                    "state": "AZ",
                    "lat": 34.8697,
                    "lon": -111.7610,
                    "type": "desert"
                }
            }
        ],
        "anthropic_api_key": anthropic_api_key,
        "openai_api_key": openai_api_key,
        "deepseek_api_key": deepseek_api_key,
        "use_weather_mcp": true, // Ensure this triggers LLM calls
        "preferred_locations": [
            {"name": "New York", "lat": 40.7128, "lon": -74.0060},
            {"name": "San Francisco", "lat": 37.7749, "lon": -122.4194}
        ],
        "states_to_monitor": ["TX", "CA", "FL", "NY", "CO", "AZ"]
    });

    // Spawn memory reverie on the first node
    let threshold = 2;
    let total_frags = 3;

    let signer = create_signer().await?;
    let verifying_public_key = signer.address();

    println!("Spawning memory reverie...");
    let reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[0].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                memory_secrets.clone(),
                threshold,
                total_frags,
                verifying_public_key
            ]
        )
    ).await??;

    println!("Memory reverie spawned on vessel: {} {}", reverie_result.id, reverie_result.description);

    // Allow time for fragment distribution
    time::sleep(Duration::from_millis(2000)).await;

    // Get the actual reverie ID from the spawn result
    let reverie_id = reverie_result.id;
    println!("Using reverie ID: {}", reverie_id);

    // Convert hash to format expected by ethers
    let digest = Keccak256::digest(reverie_id.clone().as_bytes());
    let h256_digest = ethers::types::H256::from_slice(digest.as_slice());
    let signature = signer.sign_hash(h256_digest)?;
    let signature_bytes = signature.to_vec();

    let signature_type = SignatureType::Ecdsa(signature_bytes);

    // Now execute the memory reverie from another node (node2: client[1])
    // This is the step that should trigger the LLM calls via the Python server & proxy
    println!("Executing memory reverie (this will trigger LLM calls)...");
    let exec_result: Result<(), _> = tokio::time::timeout(
        Duration::from_secs(30), // Increased timeout for LLM execution
        clients[1].request(
            "execute_with_memory_reverie",
            jsonrpsee::rpc_params![
                reverie_id,
                ReverieType::Memory,
                signature_type
            ]
        )
    ).await?;

    // Check if execution was successful
    exec_result?;
    println!("Memory reverie executed successfully via RPC.");

    // At this point, LLM calls should have been made.
    // The token usage would have been logged by the llm-proxy container.
    // You can view these logs using `docker-compose -f llm-services-compose.yml logs -f llm-proxy`
    // or by checking the ./logs/response.log file.
    println!("Check the logs of the 'llm-proxy' Docker container or './logs/response.log' for token usage details.");

    // Allow a moment for logs to flush before shutdown
    time::sleep(Duration::from_secs(2)).await;

    // Cleanup (Docker down) happens automatically via `defer!` scope guard
    Ok(())
}