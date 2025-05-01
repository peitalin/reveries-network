use reqwest::{Client, Certificate};
use serde::{Serialize, Deserialize};
use std::fs;
use std::io::Cursor;
use std::error::Error as StdError;
use chrono::Utc;
use libp2p_identity::Keypair as IdentityKeypair;
use signature::{Signer, Keypair as _};
use ed25519_dalek::SigningKey as EdSigningKey;
use base64::{Engine as _, engine::general_purpose::STANDARD as base64_standard};
use sha2::{Sha256, Digest};
use hex;
use color_eyre::eyre::{Result, anyhow};
use tracing::{info, error, warn};
use once_cell::sync::Lazy;
use crate::create_network::generate_peer_keys;
use crate::create_network::export_libp2p_public_key;
use std::env;

// Define constants for relative paths used *only* as defaults or for testing
const DEFAULT_CA_CERT_PATH_FOR_TESTS: &str = "./llm-proxy/certs/hudsucker.cer";
// Read base URL from env var, default to localhost for testing
const PROXY_API_BASE_URL: &str = "https://localhost:7070"; // Keep default for tests

// Payload definitions (mirroring llm-proxy's internal_api.rs)
#[derive(Deserialize, Serialize, Debug)]
pub struct ApiKeyPayload {
    name: String,
    key: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ApiKeyIdentifier {
    name: String,
}

fn build_api_client() -> Result<Client> {
    info!("Building standard HTTPS client trusting custom CA...");

    // Read CA path from environment variable
    let ca_cert_path = env::var("CA_CERT_PATH").unwrap_or_else(|_| {
        warn!("CA_CERT_PATH env var not set, defaulting to test path: {}", DEFAULT_CA_CERT_PATH_FOR_TESTS);
        DEFAULT_CA_CERT_PATH_FOR_TESTS.to_string()
    });

    // 1. Load CA Certificate to verify the server
    info!("Loading CA cert from: {}", ca_cert_path);
    let ca_buf = fs::read(&ca_cert_path)
        .map_err(|e| anyhow!("Failed to read CA cert file {}: {}", ca_cert_path, e))?;

    let ca_cert = Certificate::from_pem(&ca_buf)
        .map_err(|e| anyhow!("Failed to create CA certificate from PEM: {}", e))?;
    info!("CA certificate loaded successfully.");

    // 2. Build the reqwest Client
    let client = Client::builder()
        .add_root_certificate(ca_cert) // Trust our CA
        // Use default TLS backend (rustls-tls is enabled via features)
        // .tls_built_in_root_certs(false) // Optional: keep if you ONLY want to trust your CA
        .build()
        .map_err(|e| anyhow!("Failed to build HTTPS reqwest client: {}", e))?;

    info!("Standard HTTPS reqwest client built successfully.");
    Ok(client)
}

// Function to create the canonical string and signature
fn create_request_signature(
    method: &str,
    path: &str,
    timestamp: i64,
    body_bytes: &[u8],
    keypair: &IdentityKeypair,
) -> Result<(String, String)> {
    // Create canonical string
    let timestamp_str = timestamp.to_string();
    let canonical_string = llm_proxy::generate_canonical_string(
        method,
        path,
        &timestamp_str,
        body_bytes
    );
    let signature_bytes = keypair.sign(canonical_string.as_bytes())?.to_vec();
    let signature_b64 = base64_standard.encode(&signature_bytes);

    Ok((timestamp_str, signature_b64))
}

pub async fn add_proxy_api_key(
    name: String,
    key: String,
    keypair: &IdentityKeypair,
) -> Result<()> {
    let client = build_api_client()?;
    let url = format!("{}/add_api_key", PROXY_API_BASE_URL);
    let path = "/add_api_key";
    let method = "POST";
    let payload = ApiKeyPayload { name, key };

    // Serialize body just for signing hash calculation
    let body_bytes_for_sign = serde_json::to_vec(&payload)?;
    let timestamp = Utc::now().timestamp();
    let (
        timestamp_str,
        signature_b64
    ) = create_request_signature(
        method,
        path,
        timestamp,
        &body_bytes_for_sign,
        keypair
    )?;

    info!("Sending signed add_api_key request to {}", url);
    let response = client.post(&url)
        .header("X-Node-Signature", signature_b64)
        .header("X-Node-Timestamp", timestamp_str)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to send add_api_key request: {}", e))?;

    if response.status().is_success() {
        info!("Successfully added/updated API key via proxy API.");
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!("Proxy API request failed ({}): {}", status, text);
        Err(anyhow!("Failed to add API key via proxy: {} - {}", status, text))
    }
}

pub async fn remove_proxy_api_key(
    name: String,
    keypair: &IdentityKeypair,
) -> Result<()> {
    let client = build_api_client()?;
    let url = format!("{}/remove_api_key", PROXY_API_BASE_URL);
    let path = "/remove_api_key";
    let method = "POST";
    let payload = ApiKeyIdentifier { name };

    // Serialize body just for signing hash calculation
    let body_bytes_for_sign = serde_json::to_vec(&payload)?;
    let timestamp = Utc::now().timestamp();
    let (
        timestamp_str,
        signature_b64
    ) = create_request_signature(
        method,
        path,
        timestamp,
        &body_bytes_for_sign,
        keypair
    )?;

    info!("Sending signed remove_api_key request to {}", url);
    let response = client.post(&url)
        .header("X-Node-Signature", signature_b64)
        .header("X-Node-Timestamp", timestamp_str)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to send remove_api_key request: {}", e))?;

    if response.status().is_success() {
        info!("Successfully removed API key via proxy API.");
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!("Proxy API request failed ({}): {}", status, text);
        Err(anyhow!("Failed to remove API key via proxy: {} - {}", status, text))
    }
}

