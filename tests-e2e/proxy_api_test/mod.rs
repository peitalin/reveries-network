#[path = "../utils_docker.rs"]
mod utils_docker;

use std::env;
use color_eyre::Result;
use libp2p_identity::Keypair;
use once_cell::sync::Lazy;
use scopeguard::defer;
use tracing::{info, error};
use alloy_signer_local::PrivateKeySigner;
use tokio::sync::OnceCell;

use p2p_network::{
    types::AccessCondition,
    utils::pubkeys::{generate_peer_keys, export_libp2p_public_key},
    node_client::{add_proxy_api_key, remove_proxy_api_key},
};
use utils_docker::{
    init_test_logger,
    shutdown_docker_environment,
    setup_docker_environment,
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

// Lazy: once-per-module setup for proxy API tests
static TEST_SETUP: Lazy<()> = Lazy::new(|| {
    setup_test_environment_once().expect("Setup failed");
});

fn setup_test_environment_once() -> Result<()> {
    init_test_logger();
    env::set_var("CA_CERT_PATH", "../llm-proxy/certs/hudsucker.cer");

    info!("Exporting public key for seed {} to {}", TEST_KEY_SEED, TEST_NODE_PUBKEY_EXPORT_TARGET);
    if let Err(e) = export_libp2p_public_key(&TEST_KEYPAIR, TEST_NODE_PUBKEY_EXPORT_TARGET) {
        error!("Setup failed: Could not export TEST public key to {}: {}", TEST_NODE_PUBKEY_EXPORT_TARGET, e);
        return Err(e);
    }
    info!("✅ Exported test public key.");
    Ok(())
}


#[tokio::test]
#[serial_test::serial]
async fn test_add_and_remove_api_key_success() -> Result<()> {

    Lazy::force(&TEST_SETUP);

    // Start Docker Compose using the helper (async)
    if let Err(e) = setup_docker_environment(DOCKER_COMPOSE_TEST_FILE).await {
        panic!("Setup failed: Could not start Docker environment: {}", e);
    }
    defer! { shutdown_docker_environment(DOCKER_COMPOSE_TEST_FILE); } // Ensure teardown runs on scope exit

    // Test adding key
    let reverie_id = "reverie_id_TEST_KEY_REMOVE".to_string();
    let api_key = format!("test_value_{}", nanoid::nanoid!());
    // Generate a random signer and get its address
    let random_spender = PrivateKeySigner::random();
    let access_condition = AccessCondition::Ecdsa(random_spender.address());

    let add_result = add_proxy_api_key(
        reverie_id.clone(),
        "ANTHROPIC_API_KEY".to_string(),
        api_key,
        random_spender.address().to_string(),
        access_condition.get_type(),
        &TEST_KEYPAIR
    ).await;
    assert!(add_result.is_ok(), "Prerequisite add_proxy_api_key failed: {:?}", add_result.err());

    // Test removing key
    let remove_result = remove_proxy_api_key(
        reverie_id.clone(),
        &TEST_KEYPAIR
    ).await;
    assert!(remove_result.is_ok(), "remove_proxy_api_key failed: {:?}", remove_result.err());

    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn test_remove_non_existent_api_key_success() -> Result<()> {

    Lazy::force(&TEST_SETUP);
    // Start Docker Compose using the helper (async)
    if let Err(e) = setup_docker_environment(DOCKER_COMPOSE_TEST_FILE).await {
        panic!("Setup failed: Could not start Docker environment: {}", e);
    }
    defer! { shutdown_docker_environment(DOCKER_COMPOSE_TEST_FILE); } // Ensure teardown runs on scope exit

    // Test removal of non-existent key
    let name = "TEST_KEY_NON_EXISTENT_INTEGRATION".to_string();
    let _ = remove_proxy_api_key(name.clone(), &TEST_KEYPAIR).await;
    let remove_result = remove_proxy_api_key(name.clone(), &TEST_KEYPAIR).await;
    assert!(remove_result.is_err(), "remove_proxy_api_key should have failed");
    if let Err(e) = remove_result {
        assert!(e.to_string().contains("404") || e.to_string().to_lowercase().contains("not found"),
            "Error message did not indicate 'Not Found': {}", e);
    }

    Ok(())
}