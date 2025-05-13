use color_eyre::eyre::{Result, anyhow};
use std::net::SocketAddr;
use std::{
    sync::{Arc, RwLock},
    collections::HashMap,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{info, error, warn};

use axum::{
    extract::{State, ConnectInfo, Request},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json,
    Router,
    middleware::{self, Next},
    body::{Body, to_bytes},
};
use axum_server::tls_rustls::RustlsConfig;
use std::path::PathBuf;

use crate::config::{API_SERVER_CERT_PATH, API_SERVER_KEY_PATH};
use crate::config::{EnvVars, ensure_api_server_pem_files, load_or_generate_ca};

// Imports for signature verification
use ed25519_dalek::VerifyingKey as EdVerifyingKey;
use pkcs8::DecodePublicKey;
use base64::{Engine as _, engine::general_purpose::STANDARD as base64_standard};
use sha2::{Sha256, Digest};
use hex;

use std::time::Duration;
use std::path::Path;
use tokio::time::sleep as tokio_sleep; // Import async sleep
use std::env; // Add env import

// Type alias for the in-memory API key store
pub type ApiKeyStore = Arc<RwLock<HashMap<ReverieId, ApiKeyPayload>>>;

type ReverieId = String;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiKeyPayload {
    pub reverie_id: ReverieId,
    pub api_key_type: String, // ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.
    pub(crate) api_key: String,
    pub(crate) spender: String,
    pub(crate) spender_type: String,
}

impl ApiKeyPayload {
    pub fn new(
        reverie_id: ReverieId,
        api_key_type: String,
        api_key: String,
        spender: String,
        spender_type: String,
    ) -> Self {
        Self {
            reverie_id,
            api_key_type,
            api_key,
            spender,
            spender_type,
        }
    }
}

// Structure to hold shared state, including the node's public key
#[derive(Clone)]
pub struct ApiState {
    key_store: ApiKeyStore,
    node_public_key: Arc<EdVerifyingKey>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ApiKeyIdentifier {
    pub reverie_id: ReverieId,
}

// Function to load the node's public key from PEM
async fn load_node_public_key(path: &str) -> Result<EdVerifyingKey> {
    // --- Polling Logic ---
    let key_path = Path::new(path);
    let max_wait = Duration::from_secs(60);
    let poll_interval = Duration::from_secs(1);
    let start_time = std::time::Instant::now();

    info!("Waiting for node public key file at {}...", path);
    while !key_path.exists() {
        if start_time.elapsed() > max_wait {
            return Err(anyhow!("Timed out waiting for node public key file: {}", path));
        }
        info!("Key file {} not found, waiting {}s...", path, poll_interval.as_secs());
        // Use async sleep
        tokio_sleep(poll_interval).await;
    }
    info!("Found node public key file: {}", path);
    // --- End Polling Logic ---

    // Use async read_to_string
    info!("Loading p2p-node public key from {}", path);
    let pem_str = tokio::fs::read_to_string(path).await
        .map_err(|e| anyhow!("Failed to read node public key file {}: {}", path, e))?;
    let key = EdVerifyingKey::from_public_key_pem(&pem_str)
        .map_err(|e| anyhow!("Failed to decode node public key PEM: {}", e))?;
    info!("Successfully loaded and decoded p2p-node public key.");
    Ok(key)
}

pub fn generate_digest_hash(
    method: &str,
    path: &str,
    timestamp_str: &str,
    body_bytes: &[u8],
) -> String {
    // Calculate body hash
    let mut hasher = Sha256::new();
    hasher.update(body_bytes);
    let body_hash = hasher.finalize();
    let body_hash_hex = hex::encode(body_hash);

    // Create the exact canonical string the client signed
    let canonical_string = format!("{}\n{}\n{}\n{}",
        method,
        path,
        timestamp_str, // Use the original timestamp string from header
        body_hash_hex
    );

    canonical_string
}

// Updated middleware to verify node signature
async fn verify_node_request(
    State(state): State<ApiState>, // Get access to shared state
    headers: HeaderMap,
    request: Request, // Changed extractor to get full request
    next: Next,
) -> Result<Response, StatusCode> {
    info!("Verifying node request signature...");

    // 1. Extract Headers
    let signature_b64 = headers.get("X-Node-Signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            warn!("Missing X-Node-Signature header");
            StatusCode::UNAUTHORIZED
        })?;
    let timestamp_str = headers.get("X-Node-Timestamp")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            warn!("Missing X-Node-Timestamp header");
            StatusCode::UNAUTHORIZED
        })?;

    // 2. Parse Timestamp and check window
    let timestamp = timestamp_str.parse::<i64>().map_err(|_| {
        warn!("Invalid X-Node-Timestamp header: {}", timestamp_str);
        StatusCode::BAD_REQUEST
    })?;
    let now = chrono::Utc::now().timestamp();
    // Allow a time window (e.g., 60 seconds)
    if (now - timestamp).abs() > 60 {
        warn!("Timestamp outside acceptable window: {} (now={})", timestamp, now);
        return Err(StatusCode::UNAUTHORIZED); // Use FORBIDDEN or specific error?
    }

    // 3. Decode Signature
    let signature_bytes = base64_standard.decode(signature_b64).map_err(|_| {
        warn!("Invalid Base64 signature header");
        StatusCode::BAD_REQUEST
    })?;

    // Try to convert Vec<u8> to fixed-size array [u8; 64]
    let signature_array: [u8; 64] = signature_bytes.as_slice().try_into().map_err(|_| {
        warn!("Invalid signature length: expected 64 bytes, got {}", signature_bytes.len());
        StatusCode::BAD_REQUEST
    })?;

    // Now use the array with from_bytes
    let signature = ed25519_dalek::Signature::from_bytes(&signature_array); // from_bytes doesn't return Result

    // 4. Reconstruct Signed Payload
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    // Read and clone extensions and headers before consuming the request
    let extensions = request.extensions().clone();
    let original_headers = request.headers().clone(); // Clone original headers
    // Read body bytes
    let body_bytes = to_bytes(request.into_body(), usize::MAX).await.map_err(|_| {
        error!("Failed to read request body bytes for signature verification");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let canonical_string = generate_digest_hash(
        method.as_str(),
        &path,
        timestamp_str,
        &body_bytes
    );

    // 5. Verify Signature
    use signature::Verifier;
    if state.node_public_key.verify(canonical_string.as_bytes(), &signature).is_err() {
        warn!("Invalid signature for request: {} {}", method, path);
        return Err(StatusCode::UNAUTHORIZED);
    }

    info!("Node request signature verified successfully for {} {}", method, path);

    // 6. Reconstruct the request with original body, HEADERS, and extensions
    let mut request_builder = Request::builder()
        .uri(format!("http://placeholder.{}", path))
        .method(method);

    // Apply original headers to the builder
    *request_builder.headers_mut().unwrap() = original_headers;

    // Insert the cloned extensions into the new request's builder
    if let Some(ext) = request_builder.extensions_mut() {
        *ext = extensions;
    }

    let request = request_builder
        .body(Body::from(body_bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Pass the reconstructed request to the next middleware/handler
    Ok(next.run(request).await)
}

async fn add_api_key(
    State(key_store): State<ApiKeyStore>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<ApiKeyPayload>,
) -> impl IntoResponse {
    info!("Received request to add/update API key: {} from {}", &payload.reverie_id, addr);

    match key_store.write() {
        Ok(mut store) => {
            store.insert(payload.reverie_id.clone(), payload.clone());
            info!("Successfully stored API key: {}", payload.reverie_id);
            (StatusCode::OK, Json(json!({ "status": "success" })))
        }
        Err(e) => {
            error!("Failed to acquire write lock for API key store: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Internal server error" })))
        }
    }
}

async fn remove_api_key(
    State(key_store): State<ApiKeyStore>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<ApiKeyIdentifier>,
) -> impl IntoResponse {
    info!("Received request to remove API key: {} from {}", payload.reverie_id, addr);

    match key_store.write() {
        Ok(mut store) => {
            if store.remove(&payload.reverie_id).is_some() {
                info!("Successfully removed API key: {}", payload.reverie_id);
                (StatusCode::OK, Json(json!({ "status": "success" })))
            } else {
                warn!("Attempted to remove non-existent API key: {}", payload.reverie_id);
                (StatusCode::NOT_FOUND, Json(json!({ "error": "Key not found" })))
            }
        }
        Err(e) => {
            error!("Failed to acquire write lock for API key store: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "Internal server error" })))
        }
    }
}

async fn health(
    State(_key_store): State<ApiKeyStore>,
) -> impl IntoResponse {
    info!("Health check received");
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

pub async fn run_internal_api_server(key_store: ApiKeyStore, env_vars: Arc<EnvVars>) -> Result<()> {

    let addr = SocketAddr::from(([0, 0, 0, 0], env_vars.INTERNAL_API_PORT));
    info!("Internal API server attempting to start on {}", addr);

    let (ca_key_pair_for_gen, ca_cert_for_gen) = load_or_generate_ca()
        .map_err(|e| anyhow!(e))?;
    ensure_api_server_pem_files(&ca_cert_for_gen, &ca_key_pair_for_gen)?;

    info!("Loading standard TLS configuration for API server");
    let tls_config = RustlsConfig::from_pem_file(
        PathBuf::from(API_SERVER_CERT_PATH),
        PathBuf::from(API_SERVER_KEY_PATH)
    )
    .await
    .map_err(|e| anyhow!("Failed to load server TLS config from PEM files: {}", e))?;
    info!("Standard TLS configuration loaded.");

    let p2p_node_pubkey_path = env::var("P2P_NODE_PUBKEY_PATH")
        .map_err(|_| anyhow!("Missing environment variable: NODE_PUBKEY_PATH"))?;

    let p2p_node_test_pubkey_path = env::var("P2P_NODE_TEST_PUBKEY_PATH")
        .map_err(|_| anyhow!("Missing environment variable: NODE_TEST_PUBKEY_PATH"))?;

    let use_test_key = env::var("TEST_API").unwrap_or_default().to_lowercase() == "true";
    let node_pubkey  = if use_test_key {
        info!("TEST_API env var set, using test node public key path: {}", p2p_node_test_pubkey_path);
        load_node_public_key(&p2p_node_test_pubkey_path).await?
    } else {
        load_node_public_key(&p2p_node_pubkey_path).await?
    };

    let shared_state_for_auth = ApiState { // Renamed for clarity, only for auth middleware
        key_store: key_store.clone(), // Middleware might not need key_store, adjust if so
        node_public_key: Arc::new(node_pubkey),
    };

    // Router for authenticated routes
    let authed_routes = Router::new()
        .route("/add_api_key", post(add_api_key))
        .route("/remove_api_key", post(remove_api_key))
        .layer(middleware::from_fn_with_state(shared_state_for_auth, verify_node_request))
        .with_state(key_store.clone()); // Pass key_store to handlers

    // Router for unauthenticated routes (health check)
    let unauthed_routes = Router::new()
        .route("/health", get(health))
        .with_state(key_store);

    // Merge routers
    let app = Router::new()
        .merge(unauthed_routes)
        .merge(authed_routes);

    info!("Internal API server (Standard TLS) listening on https://{}", addr);
    axum_server::bind_rustls(addr, tls_config)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .map_err(|e| anyhow!("Internal API server failed: {}", e))?;

    Ok(())
}