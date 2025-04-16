#[path = "test_utils.rs"]
mod test_utils;
#[path = "network_test.rs"]
mod network_test;

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
    core::{types::TransactionRequest, utils::Anvil},
    middleware::SignerMiddleware,
    providers::{Http, Middleware, Provider},
    signers::{LocalWallet, Signer},
};
use sha3::{Digest, Keccak256};

use self::test_utils::{
    CleanupGuard,
    init_test_logger,
};
use self::network_test::start_test_network;


async fn create_signer() -> Result<LocalWallet> {
    let anvil = Anvil::new().spawn();
    let wallet: LocalWallet = anvil.keys()[0].clone().into();
    Ok(wallet)
}


#[tokio::test]
#[serial_test::serial]
async fn test_memory_reverie() -> Result<()> {

    // Define the ports we'll be using in this test
    let base_rpc_port = 8001;
    let base_listen_port = 9001;
    let num_nodes = 5;

    // Create port lists for the CleanupGuard
    let rpc_ports: Vec<u16> = (0..num_nodes).map(|i| base_rpc_port + i as u16).collect();
    let listen_ports: Vec<u16> = (0..num_nodes).map(|i| base_listen_port + i as u16).collect();
    let all_test_ports = [&rpc_ports[..], &listen_ports[..]].concat();

    // Create the guard that will clean up ports on instantiation and when it goes out of scope
    let _guard = CleanupGuard::with_ports(all_test_ports);

    // Start test network and get client connections
    let clients = start_test_network(num_nodes, base_rpc_port, base_listen_port).await?;

    // Create a test memory payload
    let memory_secrets = json!({
        "name": "First Beach Trip",
        "memory": "I remember the first time I saw the ocean. The waves were gentle, \
                  the sand was warm between my toes, and the salty breeze carried \
                  the sound of seagulls. It was a perfect summer day."
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