use reqwest::{Client, Certificate, header::HeaderMap};
use serde::{Serialize, Deserialize};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::error::Error as StdError;
use chrono::Utc;
use libp2p_identity::Keypair as IdentityKeypair;
use signature::{Signer, Keypair as _};
use ed25519_dalek::SigningKey as EdSigningKey;
use base64::{Engine as _, engine::general_purpose::STANDARD as base64_standard};
use sha2::{Sha256, Digest};
use hex;
use color_eyre::eyre::{Result, anyhow};
use tracing::{info, error, warn, debug};
use once_cell::sync::Lazy;
use crate::utils::pubkeys::{generate_peer_keys, export_libp2p_public_key};
use crate::env_var::EnvVars;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;
use tokio::time::sleep;


use llm_proxy::internal_api::{
    ApiKeyPayload,
    ApiKeyIdentifier,
};

// Reqwest client initialized only once, and reused for the node's lifetime
static PROXY_HTTP_CLIENT: OnceCell<Result<Client>> = OnceCell::const_new();
const CA_CERT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const CA_CERT_POLL_TIMEOUT: Duration = Duration::from_secs(60); // 1 minutes

async fn get_proxy_http_client() -> Result<&'static Client> {
    let client_result = PROXY_HTTP_CLIENT.get_or_init(|| async {
        info!("Attempting to initialize HTTP client for llm-proxy internal API...");
        let env_vars = EnvVars::load();
        let ca_cert_path_str = env_vars.CA_CERT_PATH;

        let ca_cert_path = Path::new(&ca_cert_path_str);
        let start_time = Instant::now();

        debug!("Polling for CA certificate at: {}", ca_cert_path.display());
        loop {
            if ca_cert_path.exists() {
                info!("CA certificate found at: {}", ca_cert_path.display());
                break;
            }
            if start_time.elapsed() > CA_CERT_POLL_TIMEOUT {
                let err_msg = format!("Timed out waiting for CA certificate at: {}", ca_cert_path.display());
                error!("{}", err_msg);
                return Err(anyhow!(err_msg));
            }
            warn!("CA certificate not found at {}, retrying in {:?}...", ca_cert_path.display(), CA_CERT_POLL_INTERVAL);
            sleep(CA_CERT_POLL_INTERVAL).await;
        }

        let ca_cert_pem = fs::read(ca_cert_path)
            .map_err(|e| anyhow!("Failed to read CA certificate from {}: {}", ca_cert_path.display(), e))?;

        let ca_cert = Certificate::from_pem(&ca_cert_pem)
            .map_err(|e| anyhow!("Failed to parse CA certificate from PEM: {}", e))?;

        info!("CA certificate loaded and parsed successfully.");

        reqwest::Client::builder()
            .add_root_certificate(ca_cert)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| {
                error!("Failed to build HTTP client for llm-proxy: {}", e);
                anyhow!("Failed to build HTTP client for llm-proxy: {}", e)
            })
    }).await;

    match client_result {
        Ok(client) => Ok(client),
        Err(e) => {
            error!("Failed to get or initialize proxy HTTP client: {}", e);
            Err(anyhow!("Failed to get or initialize proxy HTTP client: {}", e))
        }
    }
}

fn create_request_signature_headers(node_id_keypair: &IdentityKeypair, method: &str, path: &str, body_bytes: &[u8]) -> Result<HeaderMap> {
    let timestamp = chrono::Utc::now().timestamp();
    let digest_hash = llm_proxy::internal_api::generate_digest_hash(
        method,
        path,
        &timestamp.to_string(),
        body_bytes
    );

    let signature_bytes = node_id_keypair.sign(digest_hash.as_bytes())?;
    let signature_b64 = base64_standard.encode(signature_bytes);

    let mut headers = HeaderMap::new();
    headers.insert("X-Node-Signature", signature_b64.try_into()?);
    headers.insert("X-Node-Timestamp", timestamp.to_string().try_into()?);
    Ok(headers)
}

pub async fn add_proxy_api_key(
    reverie_id: String,
    api_key_type: String,
    api_key: String,
    spender: String,
    spender_type: String,
    node_id_keypair: &IdentityKeypair,
) -> Result<()> {
    info!("Attempting to add API key to proxy for reverie_id: {}", reverie_id);
    let client: &Client = get_proxy_http_client().await?;
    let env_vars = EnvVars::load();
    let proxy_internal_api_url = format!(
        "{}{}",
        env_vars.PROXY_API_BASE_URL,
        "/add_api_key"
    );

    let payload = ApiKeyPayload::new(reverie_id.clone(), api_key_type, api_key, spender, spender_type);
    let body_bytes = serde_json::to_vec(&payload)?;

    let signature_headers = create_request_signature_headers(node_id_keypair, "POST", "/add_api_key", &body_bytes)?;

    let response = client
        .post(&proxy_internal_api_url)
        .headers(signature_headers)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to send add_api_key request to proxy: {}", e))?;

    if response.status().is_success() {
        info!("Successfully added API key via proxy for reverie_id: {}", payload.reverie_id);
        Ok(())
    } else {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!(
            "Failed to add API key via proxy. Status: {}. Body: {}",
            status, error_body
        );
        Err(anyhow!(
            "Failed to add API key via proxy. Status: {}. Body: {}",
            status, error_body
        ))
    }
}

pub async fn remove_proxy_api_key(reverie_id: String, node_id_keypair: &IdentityKeypair) -> Result<()> {
    info!("Attempting to remove API key from proxy for reverie_id: {}", reverie_id);
    let client = get_proxy_http_client().await?;
    let env_vars = EnvVars::load();
    let proxy_internal_api_url = format!(
        "{}{}",
        env_vars.PROXY_API_BASE_URL,
        "/remove_api_key"
    );

    let payload = ApiKeyIdentifier { reverie_id: reverie_id.clone() };
    let body_bytes = serde_json::to_vec(&payload)?;

    let signature_headers = create_request_signature_headers(node_id_keypair, "POST", "/remove_api_key", &body_bytes)?;

    let response = client
        .post(&proxy_internal_api_url)
        .headers(signature_headers)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to send remove_api_key request to proxy: {}", e))?;

    if response.status().is_success() {
        info!("Successfully removed API key via proxy for reverie_id: {}", reverie_id);
        Ok(())
    } else {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_else(|_| "Failed to read error body".to_string());
        error!(
            "Failed to remove API key via proxy. Status: {}. Body: {}",
            status, error_body
        );
        Err(anyhow!(
            "Failed to remove API key via proxy. Status: {}. Body: {}",
            status, error_body
        ))
    }
}

