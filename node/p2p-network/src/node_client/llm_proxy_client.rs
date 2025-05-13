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
use crate::utils::pubkeys::generate_peer_keys;
use crate::env_var::EnvVars;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;
use tokio::time::sleep;

use llm_proxy::api_key_delegation_server::{
    ApiKeyPayload,
    ApiKeyIdentifier,
};

use super::NodeClient;

// Reqwest client initialized only once, and reused for the node's lifetime
static PROXY_HTTP_CLIENT: OnceCell<Result<Client>> = OnceCell::const_new();
const CA_CERT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const CA_CERT_POLL_TIMEOUT: Duration = Duration::from_secs(60); // 1 minutes

async fn get_proxy_http_client(ca_cert: reqwest::Certificate) -> Result<&'static Client> {
    let client_result = PROXY_HTTP_CLIENT.get_or_init(|| async {

        info!("Initializing HTTP client for llm-proxy internal API...");

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
    let digest_hash = llm_proxy::api_key_delegation_server::generate_digest_hash(
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

impl NodeClient {

    pub async fn add_proxy_api_key(
        &mut self,
        reverie_id: String,
        api_key_type: String,
        api_key: String,
        spender: String,
        spender_type: String,
    ) -> Result<()> {

        println!("Attempting to add API key to proxy for reverie_id: {}", reverie_id);

        let ca_cert= match self.llm_proxy_ca_cert.read().await.clone() {
            Some(pem) => pem,
            None => return Err(anyhow!("No CA certificate PEM found in NodeClient. Delegation failed.")),
        };

        let node_id_keypair = &self.node_id.id_keys.clone();

        let client: &Client = get_proxy_http_client(ca_cert).await?;
        let env_vars = EnvVars::load();
        let proxy_internal_api_url = format!(
            "{}{}",
            env_vars.LLM_PROXY_API_URL,
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
            .map_err(|e| {
                let mut err_msg = format!("Failed to send add_api_key request to proxy. Reqwest error: {}", e);
                if let Some(source) = e.source() {
                    err_msg.push_str(&format!("\n  Caused by: {}", source));
                    if let Some(source2) = source.source() {
                        err_msg.push_str(&format!("\n    Further caused by: {}", source2));
                    }
                }
                error!("add_proxy_api_key: {}", err_msg); // Log detailed error
                anyhow!(err_msg)
            })?;

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

    pub async fn remove_proxy_api_key(
        &mut self,
        reverie_id: String,
    ) -> Result<()> {
        println!("removing API key from proxy for reverie_id: {}", reverie_id);
        let ca_cert= match self.llm_proxy_ca_cert.read().await.clone() {
            Some(pem) => pem,
            None => return Err(anyhow!("No CA certificate PEM found in NodeClient. Delegation failed.")),
        };

        let node_id_keypair = &self.node_id.id_keys.clone();

        let client = get_proxy_http_client(ca_cert).await?;
        let env_vars = EnvVars::load();
        let proxy_internal_api_url = format!(
            "{}{}",
            env_vars.LLM_PROXY_API_URL,
            "/remove_api_key"
        );

        let payload = ApiKeyIdentifier { reverie_id: reverie_id.clone() };
        let body_bytes = serde_json::to_vec(&payload)?;

        // Log the JSON body that will be sent
        match std::str::from_utf8(&body_bytes) {
            Ok(json_str) => error!("REMOVE_PROXY_API_KEY_DEBUG: Sending JSON body: {}", json_str),
            Err(_) => error!("REMOVE_PROXY_API_KEY_DEBUG: Body for signature is not valid UTF-8 (length: {})", body_bytes.len()),
        }

        let signature_headers = create_request_signature_headers(
            node_id_keypair,
            "POST",
            "/remove_api_key",
            &body_bytes
        )?;

        let response = client
            .post(&proxy_internal_api_url)
            .headers(signature_headers)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let mut err_msg = format!("Failed to send remove_api_key request to proxy. Reqwest error: {}", e);
                if let Some(source) = e.source() {
                    err_msg.push_str(&format!("\n  Caused by: {}", source));
                    if let Some(source2) = source.source() {
                        err_msg.push_str(&format!("\n    Further caused by: {}", source2));
                    }
                }
                error!("remove_proxy_api_key: {}", err_msg); // Log detailed error
                anyhow!(err_msg)
            })?;

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

}