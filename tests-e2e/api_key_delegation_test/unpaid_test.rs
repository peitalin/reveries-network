#[path = "../utils_docker.rs"]
mod utils_docker;
#[path = "../utils_network.rs"]
mod utils_network;

use alloy_primitives::B256;
use alloy_signer::Signer;
use color_eyre::{Result, eyre::Error, eyre::anyhow};
use jsonrpsee::core::client::ClientT;
use std::time::Duration;
use scopeguard::defer;
use serde_json::json;
use sha3::{Digest, Keccak256};
use tokio::time;

// internal imports
use p2p_network::types::{
    Reverie,
    ReverieType,
    AccessKey,
    AccessCondition,
    ExecuteWithMemoryReverieResult,
    create_digest_hash,
};
use utils_docker::shutdown_docker_environment;
use utils_network::TestNodes;
use crate::*;

//////////////////////////////////////
//// Access Condition: ECDSA Signature Test
//////////////////////////////////////

#[tokio::test]
#[serial_test::serial]
pub async fn test_api_key_delegation_ecdsa() -> Result<()> {

    setup_test_environment_once_async().await?;

    let test_nodes = TestNodes::new(5)
        .exec_docker_compose_with_node(
            TARGET_NODE_2,
            format!("docker-compose -f {} up -d",  DOCKER_COMPOSE_TEST_FILE)
        )
        .start_test_network().await?
        .create_rpc_clients().await?
        .wait_for_llm_proxy_key_registration(TARGET_NODE_2).await?;

    let clients = test_nodes.rpc_clients.clone();

    /////////////////////////////////////////////////////////
    //// Access Conditions
    /////////////////////////////////////////////////////////

    // Dev's signer and public key
    let signer_dev = create_signer().await?;
    let access_condition_dev = AccessCondition::Ecdsa(signer_dev.address());
    // User's signer and public key
    let signer_user = create_signer().await?;
    let access_condition_user = AccessCondition::Ecdsa(signer_user.address());

    let threshold = 2;
    let total_frags = 3;

    /////////////////////////////////////////////////////////
    //// Encrypt Secret #1: Anthropic API key
    /////////////////////////////////////////////////////////
    println!("Step 1: Spawn API Key Reverie with encrypted Anthropic API key...");
    let api_keys_secrets = json!({
        "anthropic_api_key": std::env::var("ANTHROPIC_API_KEY").ok(),
        "deepseek_api_key": std::env::var("DEEPSEEK_API_KEY").ok(),
    });
    let api_key_reverie: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&9901].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                api_keys_secrets,
                threshold,
                total_frags,
                access_condition_dev
            ]
        )
    ).await??;

    println!("API Key reverie spawned on vessel: {} {}", api_key_reverie.id, api_key_reverie.description);
    time::sleep(Duration::from_millis(2000)).await;

    println!("Step 2: Delegate Anthropic API key to dev's server...");
    let access_key_dev = {
        let digest = Keccak256::digest(api_key_reverie.id.clone().as_bytes());
        let hash = B256::from_slice(digest.as_slice());
        let signature = signer_dev.sign_hash(&hash).await?;
        AccessKey::EcdsaSignature(signature.as_bytes().to_vec())
    };
    clients[&9902].request(
        "delegate_api_key",
        jsonrpsee::rpc_params![
            api_key_reverie.id,
            ReverieType::Memory,
            access_key_dev
        ]
    ).await?;

    /////////////////////////////////////////////////////////
    //// Encrypt Secret #2: Memories
    /////////////////////////////////////////////////////////
    println!("Step 3: Delegate secret context/memories...");
    let memory_secrets = TEST_MEMORY_REVERIE.get().unwrap();
    let memory_reverie_result: Reverie = tokio::time::timeout(
        Duration::from_secs(5),
        clients[&9901].request(
            "spawn_memory_reverie",
            jsonrpsee::rpc_params![
                memory_secrets.clone(),
                threshold,
                total_frags,
                access_condition_user
            ]
        )
    ).await??;

    println!("Memory reverie spawned on vessel: {} {}", memory_reverie_result.id, memory_reverie_result.description);
    time::sleep(Duration::from_millis(1000)).await;

    println!("Step 4: Execute LLM calls with secret memory and delegated API Key");

    let access_key_user = {
        let digest = Keccak256::digest(memory_reverie_result.id.clone().as_bytes());
        let hash = B256::from_slice(digest.as_slice());
        // TODO: add nonce and timestamp to digestHash
        let hash2 = create_digest_hash(&memory_reverie_result.id, 0, 0);
        assert_eq!(hash, hash2);
        let signature = signer_user.sign_hash(&hash).await?;
        AccessKey::EcdsaSignature(signature.as_bytes().to_vec())
    };

    let anthropic_prompt = TEST_ANTHROPIC_PROMPT.get().unwrap();

    let rpc_result = tokio::time::timeout(
        Duration::from_secs(20), // Increased timeout for LLM tool-use calls
        clients[&9902].request::<ExecuteWithMemoryReverieResult, _>(
            "execute_with_memory_reverie",
            jsonrpsee::rpc_params![
                memory_reverie_result.id,
                ReverieType::Memory,
                access_key_user,
                anthropic_prompt
            ]
        )
    ).await?;

    println!("Memory reverie executed successfully via RPC.");
    match rpc_result {
        Err(e) => println!("Error:\n{:?}", e),
        Ok(result) => {
            println!("\nClaude Result: {:?}", result.claude);
            println!("Usage Report: {:?}\n", result.usage_report);
        }
    };

    time::sleep(Duration::from_secs(1)).await;
    defer! { test_nodes.cleanup_ports(); }
    defer! { shutdown_docker_environment(3); }
    Ok(())
}
