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
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;
use tokio::time::sleep;
use p256::ecdsa::{
    VerifyingKey as P256VerifyingKey,
    Signature as P256Signature
};
use ecdsa::signature::Verifier;
use elliptic_curve::pkcs8::DecodePublicKey;
use crate::utils::pubkeys::generate_peer_keys;
use crate::env_var::EnvVars;
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

    pub async fn register_llm_proxy_pubkey(
        &mut self,
        payload: llm_proxy::types::LlmProxyPublicKeyPayload,
    ) -> Result<String> {
        info!("NodeClient: Processing LLM Proxy public key registration...");

        // 1. Parse the PEM public key string to a P256VerifyingKey
        let llm_proxy_pubkey = P256VerifyingKey::from_public_key_pem(&payload.pubkey_pem)
            .map_err(|e| anyhow!("Failed to parse LLM Proxy public key PEM: {}", e))?;
        debug!("NodeClient: Successfully parsed LLM Proxy public key PEM.");

        // 2. Decode the Base64 signature
        let signature_bytes = base64_standard.decode(&payload.signature_b64)
            .map_err(|e| anyhow!("Failed to decode Base64 signature for LLM Proxy key: {}", e))?;
        let signature = P256Signature::from_slice(&signature_bytes)
            .map_err(|e| anyhow!("Failed to parse signature bytes for LLM Proxy key: {}", e))?;
        debug!("NodeClient: Successfully decoded and parsed signature.");

        // 3. Verify the signature
        // The signature was made over the PEM string of the public key itself.
        llm_proxy_pubkey.verify(payload.pubkey_pem.as_bytes(), &signature)
            .map_err(|e| anyhow!("LLM Proxy public key signature verification failed: {}", e))?;
        println!("NodeClient: LLM Proxy public key signature VERIFIED.");

        // 4. Store the verified public key
        let _ = self.llm_proxy_public_key.write().await.insert(llm_proxy_pubkey);

        // 5. Store the CA certificate PEM string
        info!("NodeClient: Storing LLM Proxy CA certificate PEM.\nPEM:\n{}", payload.ca_cert_pem);
        let ca_cert = reqwest::Certificate::from_pem(payload.ca_cert_pem.as_bytes())?;
        let _ = self.llm_proxy_ca_cert.write().await.insert(ca_cert);
        info!("NodeClient: Successfully stored LLM Proxy CA certificate PEM.");

        println!("Stored LLM Proxy public key: {:?}", &self.llm_proxy_public_key.read().await);
        println!("Stored LLM Proxy CA certificate PEM: {:?}", &self.llm_proxy_ca_cert.read().await);

        let pubkey_log = self.llm_proxy_public_key.read().await.clone();
        let ca_cert_pem_log = self.llm_proxy_ca_cert.read().await.clone(); // Log the PEM string
        let pubkey_log_str = format!("{:?}", pubkey_log);
        let ca_cert_pem_log_str = format!("{:?}", ca_cert_pem_log); // Debug format of Option<String>

        Ok(serde_json::json!({
            "llm_proxy_public_key": pubkey_log_str,
            "llm_proxy_ca_cert_pem": ca_cert_pem_log_str, // Updated field name in log
        }).to_string())
    }

    pub async fn get_proxy_public_key(&self) -> Result<(Option<String>, Option<String>)> {
        let proxy_public_key = self.llm_proxy_public_key.read().await;
        let opt_public_key = proxy_public_key.as_ref();
        let public_key_str = match opt_public_key {
            Some(ref key) => Some(format!("{:?}", key)),
            None => None
        };

        let ca_cert = self.llm_proxy_ca_cert.read().await;
        let opt_ca_cert = ca_cert.as_ref();
        let ca_cert_str = match opt_ca_cert {
            Some(ref cert) => Some(format!("{:?}", cert)),
            None => None
        };
        Ok((public_key_str, ca_cert_str))
    }

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