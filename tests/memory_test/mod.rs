#[path = "../test_utils.rs"]
mod test_utils;
#[path = "../network_utils.rs"]
mod network_utils;

use std::time::Duration;
use color_eyre::Result;
use jsonrpsee::core::client::ClientT;
use tokio::time;
use serde_json::json;
use tracing::info;

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
use self::network_utils::{start_test_network, start_python_server};


async fn create_signer() -> Result<LocalWallet> {
    let anvil = Anvil::new().spawn();
    let wallet: LocalWallet = anvil.keys()[0].clone().into();
    Ok(wallet)
}


#[tokio::test]
#[serial_test::serial]
pub async fn test_memory_reverie() -> Result<()> {

    // Define the ports we'll be using in this test
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;
    let python_server_port = 8000;

    // Create port lists for the CleanupGuard
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let mut all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Add Python server port to the cleanup list
    all_test_ports.push(python_server_port);

    // Create the guard that will clean up ports on instantiation and when it goes out of scope
    let _guard = CleanupGuard::with_ports(all_test_ports);

    // Start Python FastAPI server - this server handles LLM API requests
    // and is used by the memory_reverie code to query language models
    info!("Starting Python FastAPI server for LLM integrations...");
    let _python_server = start_python_server().await?;
    info!("Python FastAPI server started successfully and ready to process requests");

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;


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
            },
            {
                "name": "Lake Effect Snow",
                "memory": "I woke to a world transformed. The lake effect snow had fallen all night, \
                    silent and steady, wrapping everything in a pristine blanket of white. The trees \
                    outside my window bent under the weight of it, and the usual sounds of the city \
                    were muffled. It was as if someone had turned down the volume on the world.",
                "location": {
                    "name": "Buffalo",
                    "state": "NY",
                    "lat": 42.8864,
                    "lon": -78.8784,
                    "type": "lakeside"
                }
            }
        ],
        "anthropic_api_key": anthropic_api_key,
        "openai_api_key": openai_api_key,
        "deepseek_api_key": deepseek_api_key,
        "use_weather_mcp": true,
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
    // Example Ethereum address as verifying key

    info!("Spawning memory reverie...");
    let reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(10),
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
    // should match the signature verification in
    let digest = Keccak256::digest(reverie_id.clone().as_bytes());
    let h256_digest = ethers::types::H256::from_slice(digest.as_slice());
    let signature = signer.sign_hash(h256_digest)?;
    let signature_bytes = signature.to_vec();

    let signature_type = SignatureType::Ecdsa(signature_bytes);

    // Now execute the memory reverie from another node (client[1])
    println!("Executing memory reverie...");
    clients[1].request(
        "execute_with_memory_reverie",
        jsonrpsee::rpc_params![
            reverie_id,
            ReverieType::Memory,
            signature_type
        ]
    ).await?;

    info!("Memory reverie executed successfully");

    // Cleanup happens automatically via CleanupGuard drop
    Ok(())
}