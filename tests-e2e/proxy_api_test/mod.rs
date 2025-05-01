#[path = "../test_utils.rs"]
mod test_utils;

use std::env;
use color_eyre::Result;
use libp2p_identity::Keypair;
use once_cell::sync::Lazy;
use scopeguard::defer;
use tracing::{info, error};

use p2p_network::node_client::{add_proxy_api_key, remove_proxy_api_key};
use p2p_network::create_network::{generate_peer_keys, export_libp2p_public_key};
use test_utils::{
    init_test_logger,
    shutdown_docker_environment,
    setup_docker_environment,
};

const DOCKER_COMPOSE_TEST_FILE: &str = "../docker-compose-llm-proxy-test.yml";
// Path for the test key export relative to /tests dir
const TEST_NODE_PUBKEY_EXPORT_TARGET: &str = "../llm-proxy/pubkeys/p2p-node/p2p_node_test.pub.pem";
// Path for test setup relative to tests dir
const TEST_KEY_SEED: usize = 2; // any seed between 1 and 5
// Generate a FIXED keypair using seed
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
    setup();
});

fn ensure_setup() {
    env::set_var("CA_CERT_PATH", "../llm-proxy/certs/hudsucker.cer");
    // Force initialization of the Lazy static
    Lazy::force(&TEST_SETUP);
}

fn setup() {
    init_test_logger();
    // 1. Export the test public key
    info!("Exporting public key for seed {} to {}", TEST_KEY_SEED, TEST_NODE_PUBKEY_EXPORT_TARGET);
    if let Err(e) = export_libp2p_public_key(&TEST_KEYPAIR, TEST_NODE_PUBKEY_EXPORT_TARGET) {
        panic!("Setup failed: Could not export TEST public key to {}: {}", TEST_NODE_PUBKEY_EXPORT_TARGET, e);
    }
    info!("✅ Exported test public key.");

    // 2. Start Docker Compose using the helper
    if let Err(e) = setup_docker_environment(DOCKER_COMPOSE_TEST_FILE) {
        // Use panic! here because setup is expected to succeed or the tests are invalid
        panic!("Setup failed: Could not start Docker environment: {}", e);
    }
    info!("✅ Setup complete.");
}


#[tokio::test]
#[serial_test::serial]
async fn test_add_and_remove_api_key_success() -> Result<()> {
    ensure_setup();
    defer! { shutdown_docker_environment(DOCKER_COMPOSE_TEST_FILE); } // Ensure teardown runs on scope exit

    // Test adding key
    let name = "TEST_KEY_REMOVE_INTEGRATION".to_string();
    let key = format!("test_value_{}", nanoid::nanoid!());
    let add_result = add_proxy_api_key(name.clone(), key, &TEST_KEYPAIR).await;
    assert!(add_result.is_ok(), "Prerequisite add_proxy_api_key failed: {:?}", add_result.err());

    // Test removing key
    let remove_result = remove_proxy_api_key(name.clone(), &TEST_KEYPAIR).await;
    assert!(remove_result.is_ok(), "remove_proxy_api_key failed: {:?}", remove_result.err());

    Ok(())
}

#[tokio::test]
#[serial_test::serial]
async fn test_remove_non_existent_api_key_success() -> Result<()> {
    ensure_setup();
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