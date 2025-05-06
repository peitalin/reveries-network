mod parser;
mod tee_body_sse;
mod tee_body;
mod usage;
mod config;
mod internal_api;

use std::{
    net::SocketAddr,
    error::Error as StdError,
    sync::{Arc, RwLock},
    collections::HashMap,
};
use chrono::Utc;
use hudsucker::{
    hyper::{self, Request, Response, header::{HeaderName, HeaderValue}},
    rustls::crypto::aws_lc_rs,
    rustls::crypto::CryptoProvider,
    tokio_tungstenite::tungstenite::Message,
    Proxy, HttpHandler, HttpContext, RequestOrResponse, Body, WebSocketHandler, WebSocketContext,
    certificate_authority::RcgenAuthority,
};
use serde::{Deserialize, Serialize};
use http_body_util::{Full, BodyExt};
use hyper::header::CONTENT_TYPE;
use tracing::{debug, error, info, warn};
use p256::ecdsa::SigningKey;
use serde_json::Value;
use rand::seq::SliceRandom;
use rand::thread_rng;
use sha2::{Sha256, Digest};
use base64::{engine::general_purpose::STANDARD as base64_standard};

use crate::config::{
    EnvVars,
    load_or_generate_signing_key,
    load_or_generate_ca,
    ensure_api_server_pem_files
};
use crate::usage::{log_sse_response_task, log_regular_response_task};
use crate::internal_api::{run_internal_api_server, ApiKeyStore};


static ANTHROPIC_DELEGATE_API_KEY: &str = "sk-ant-delegated-api-key";
static ANTHROPIC_DELEGATE_API_KEY_VALUE: &str = "sk-ant-delegated-api-key";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LLMProxyRequestContext {
    request_id: String,
    request_url: Option<String>,
    linked_tool_use_id: Option<String>,
    reverie_id: Option<String>,
    spender: Option<String>,
    spender_type: Option<String>,
}

#[derive(Clone)]
struct LogHandler {
    signing_key: Arc<SigningKey>,
    env: Arc<EnvVars>,
    api_key_store: ApiKeyStore,
}

// Helper function for safe logging of API keys
fn log_api_key_info(key_name: &str, api_key: &str, request_id: &str) {
    let key_len = api_key.len();
    let prefix = api_key.chars().take(5).collect::<String>();
    let suffix = api_key.chars().skip(key_len.saturating_sub(4)).collect::<String>();
    info!(
        "Request {}: Injecting API key '{}': {}...{}",
        request_id, key_name, prefix, suffix
    );
}

impl HttpHandler for LogHandler {
    async fn handle_request(
        &mut self,
        ctx: &mut HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        let (mut parts, body) = req.into_parts();
        let request_id = create_request_id();
        let url = parts.uri.to_string();
        let mut reverie_id_for_context: Option<String> = None;
        let mut spender_for_context: Option<String> = None;
        let mut spender_type_for_context: Option<String> = None;

        info!("===== Intercepted Request {}: {} ====", request_id, url);
        info!("Request {}: Method: {}", request_id, parts.method);
        for (name, value) in &parts.headers {
            info!("Request {}: Header: {} = {:?}", request_id, name, value);
        }

        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Request {}: Failed to collect request body: {}", request_id, e);
                return Request::from_parts(parts, Body::empty()).into();
            }
        };

        // -- Start API Key Injection Logic --
        debug!(
            "Request {}: Checking URI: \"{}\", Host: {:?}",
            request_id,
            parts.uri,
            parts.uri.host()
        );
        if let Some(host) = parts.uri.host() {
            if host.contains("anthropic.com") {
                debug!("Request {}: Detected Anthropic API request.", request_id);
                let x_api_key_header_value = parts.headers.get(HeaderName::from_static("x-api-key"));

                if is_anthropic_delegation_request(x_api_key_header_value) {
                    let store = self.api_key_store.read().expect("API key store lock poisoned");
                    let anthropic_keys: Vec<_> = store.values()
                        .filter(|payload| payload.api_key_type.eq_ignore_ascii_case("ANTHROPIC_API_KEY"))
                        .collect();

                    if anthropic_keys.is_empty() {
                        warn!("Request {}: Proxy injection failed: No Anthropic keys found in store for delegation.", request_id);
                    } else {
                        if let Some(&selected_payload) = anthropic_keys.choose(&mut thread_rng()) {
                            let api_key = &selected_payload.api_key;
                            reverie_id_for_context = Some(selected_payload.reverie_id.clone());
                            spender_for_context = Some(selected_payload.spender.clone());
                            spender_type_for_context = Some(selected_payload.spender_type.clone());

                            match HeaderValue::from_str(api_key) {
                                Ok(header_value) => {
                                    log_api_key_info(&selected_payload.reverie_id, api_key, &request_id);
                                    parts.headers.insert(HeaderName::from_static("x-api-key"), header_value);
                                }
                                Err(e) => {
                                    error!("Request {}: Failed to create HeaderValue for selected Anthropic API key (Reverie: {}): {}", request_id, selected_payload.reverie_id, e);
                                }
                            }
                        } else {
                            warn!("Request {}: Failed to randomly select an Anthropic key even though keys were found for delegation.", request_id);
                        }
                    }
                }
            }
        }
        // -- End API Key Injection Logic --

        if let Ok(body_str) = std::str::from_utf8(&body_bytes) {
            info!("Request {}: Body:\n{}", request_id, body_str);
        } else {
            info!("Request {}: Body: <Non-UTF8 data: {} bytes>", request_id, body_bytes.len());
        }

        let mut request_context = LLMProxyRequestContext {
            request_id: request_id.clone(),
            request_url: Some(url.clone()),
            linked_tool_use_id: None,
            reverie_id: reverie_id_for_context,
            spender: spender_for_context,
            spender_type: spender_type_for_context,
        };
        if let Some(found_tool_use_id) = find_tool_use_id_in_request_body(&body_bytes) {
             info!("Request {}: Found linked tool_use_id: {}", request_id, found_tool_use_id);
            request_context.linked_tool_use_id = Some(found_tool_use_id);
        }
        if let Err(e) = ctx.set_request_context(request_context) {
            error!("Request {}: Failed to set request context: {}", request_id, e);
        };

        Request::from_parts(parts, Body::from(Full::new(body_bytes))).into()
    }

    async fn handle_response(
        &mut self,
        ctx: &mut HttpContext,
        res: Response<Body>
    ) -> Response<Body> {
        let (parts, body) = res.into_parts();

        let (
            request_id,
            request_url,
            linked_tool_use_id,
            reverie_id,
            spender,
            spender_type,
        ) = if let Some(context) = &ctx.request_context {
            let request_context = serde_json::from_value::<LLMProxyRequestContext>(context.clone()).unwrap();
            info!("Response {}: Retrieved context from HttpContext field", request_context.request_id);
            (
                request_context.request_id.clone(),
                request_context.request_url.clone(),
                request_context.linked_tool_use_id.clone(),
                request_context.reverie_id.clone(),
                request_context.spender.clone(),
                request_context.spender_type.clone(),
            )
        } else {
            warn!("Response: Could not retrieve request context from HttpContext field! Generating new ID.");
            (create_request_id(), None, None, None, None, None)
        };

        info!("Response for {}: Intercepted response with status: {}", request_id, parts.status);
        debug!("Response {}: Using linked_tool_use_id: {:?}", request_id, linked_tool_use_id);

        let content_type = parts.headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok());
        let is_sse = content_type.map_or(false, |ct| ct.starts_with("text/event-stream"));
        let headers_for_log = parts.headers.clone();
        let key_arc = self.signing_key.clone();
        let report_url = self.env.report_usage_url.clone();

        let response_body;
        if is_sse {
            info!("Response {}: SSE stream detected, using SSE logging task.", request_id);
            let (teed_body, receiver) = crate::tee_body_sse::tee_body_sse(body, request_url.as_deref());
            tokio::spawn(log_sse_response_task(
                receiver,
                headers_for_log,
                key_arc,
                request_url,
                linked_tool_use_id,
                reverie_id,
                report_url,
                request_id.clone(),
                spender,
                spender_type,
            ));
            response_body = teed_body;
        } else {
            info!("Response {}: Non-SSE response detected, using full body logging task.", request_id);
            let (teed_body, receiver) = crate::tee_body::tee_body(body);
            tokio::spawn(log_regular_response_task(
                receiver,
                headers_for_log,
                key_arc,
                request_url,
                linked_tool_use_id,
                reverie_id,
                report_url,
                request_id.clone(),
                spender,
                spender_type,
            ));
            response_body = teed_body;
        }

        info!("Response {}: [{}] Returning teed response to client...", request_id, Utc::now().to_rfc3339());
        Response::from_parts(parts, response_body)
    }
}

impl WebSocketHandler for LogHandler {
    async fn handle_message(&mut self, _ctx: &WebSocketContext, msg: Message) -> Option<Message> {
        info!("WebSocket message: {:?}", msg);
        Some(msg)
    }
}

fn is_anthropic_delegation_request(option_header_value: Option<&HeaderValue>) -> bool {
    let mut needs_delegation = false;
    if let Some(header_api_key_value) = option_header_value {
        if header_api_key_value == ANTHROPIC_DELEGATE_API_KEY_VALUE {
            needs_delegation = true;
            debug!("Request: x-api-key header is the delegation key value, attempting injection from store.");
        } else {
            debug!("Request: x-api-key header present but not the delegation key value ('{:?}'), skipping direct injection attempt based on this.", header_api_key_value);
        }
    } else {
        debug!("Request: No x-api-key header found. This request does not use explicit delegation via x-api-key.");
    };
    return needs_delegation
}

fn find_tool_use_id_in_request_body(body_bytes: &[u8]) -> Option<String> {
    let json_body: Value = serde_json::from_slice(body_bytes).ok()?;
    let messages = json_body.get("messages")?.as_array()?;
    for (msg_idx, msg) in messages.iter().enumerate() {
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => debug!("find_tool_use_id: Found message with role 'user' at index {}.", msg_idx),
            _ => continue,
        }
        let content_array = match msg.get("content").and_then(Value::as_array) {
            Some(arr) => arr,
            None => continue,
        };
        for item in content_array.iter() {
            match item.get("type").and_then(Value::as_str) {
                Some("tool_result") => {
                    match item.get("tool_use_id").and_then(Value::as_str) {
                        Some(id_str) => return Some(id_str.to_string()),
                        None => warn!("find_tool_use_id: 'tool_result' item missing string 'tool_use_id' field."),
                    }
                },
                _ => continue,
            }
        }
    }
    None
}

fn create_request_id() -> String {
    format!("request_{}", nanoid::nanoid!())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError + Send + Sync>> {

    // Install the crypto provider first
    CryptoProvider::install_default(aws_lc_rs::default_provider()).ok();

    tracing_subscriber::fmt()
        .without_time()
        .init();

    info!("Starting LLM proxy...");

    let env_vars = Arc::new(EnvVars::load());

    // Initialize the in-memory API key store
    let api_key_store: ApiKeyStore = Arc::new(RwLock::new(HashMap::new()));

    // Load/Generate CA first
    let (ca_key_pair, ca_cert) = load_or_generate_ca()?;
    // Ensure API server PEM files exist using the CA
    ensure_api_server_pem_files(&ca_cert, &ca_key_pair)?;

    // Load usage signing key
    let signing_key = load_or_generate_signing_key()?;
    let signing_key_arc = Arc::new(signing_key);

    // Clone Arcs for the internal API server task
    let api_key_store_clone = api_key_store.clone();
    let env_vars_clone = env_vars.clone();
    // Spawn the internal API server task
    tokio::spawn(async move {
        if let Err(e) = run_internal_api_server(api_key_store_clone, env_vars_clone).await {
            error!("Internal API server failed: {}", e);
        }
    });


    // Hudsucker CA setup uses the same CA key/cert
    let hudsucker_ca = RcgenAuthority::new(
        ca_key_pair, // Use loaded/generated CA key
        ca_cert,     // Use loaded/generated CA cert
        1_000,
        aws_lc_rs::default_provider()
    );

    // Adjust LogHandler initialization (no DB needed)
    let proxy = Proxy::builder()
        .with_addr(SocketAddr::from(([0, 0, 0, 0], 8080)))
        .with_ca(hudsucker_ca) // Use the loaded/generated CA
        .with_rustls_client(aws_lc_rs::default_provider())
        .with_http_handler(LogHandler {
            signing_key: signing_key_arc.clone(),
            env: env_vars.clone(),
            api_key_store: api_key_store.clone(),
        })
        .with_websocket_handler(LogHandler {
            signing_key: signing_key_arc.clone(),
            env: env_vars.clone(),
            api_key_store: api_key_store.clone(),
        })
        .with_graceful_shutdown(shutdown_signal())
        .build()
        .map_err(|e| format!("Failed to build proxy: {}", e))?;

    let db_mode = "memory";
    info!("llm-proxy listening on 0.0.0.0:8080 (DB mode: {})", db_mode);
    proxy.start().await?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("Failed to install CTRL+C signal handler: {}", e))
        .expect("Ctrl-C handler setup failed");
}