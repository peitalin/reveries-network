mod parser;
mod tee_body_sse;
mod tee_body;
mod usage;
mod request_context_db;
mod config;

use std::{
    net::SocketAddr,
    error::Error as StdError,
    sync::Arc,
    time::Duration,
    sync::Mutex,
    collections::HashMap,
};
use chrono::Utc;
use hudsucker::{
    hyper::{self, Request, Response},
    rustls::crypto::aws_lc_rs,
    tokio_tungstenite::tungstenite::Message,
    Proxy, HttpHandler, HttpContext, RequestOrResponse, Body, WebSocketHandler, WebSocketContext,
    certificate_authority::RcgenAuthority,
};
use http_body_util::{Full, BodyExt};
use hyper::header::CONTENT_TYPE;
use tracing::{debug, error, info, trace, warn};
use p256::ecdsa::SigningKey;
use serde_json::Value;

use crate::config::{
    EnvVars,
    load_or_generate_signing_key,
    load_or_generate_ca,
};
use crate::usage::{log_sse_response_task, log_regular_response_task};
use crate::request_context_db::{init_db, DbPool, cleanup_old_contexts};


#[derive(Clone)]
struct LogHandler {
    signing_key: Arc<SigningKey>,
    db_pool: DbPool,
    env: Arc<EnvVars>,
    request_ids: Arc<Mutex<HashMap<SocketAddr, String>>>,
}

impl HttpHandler for LogHandler {
    async fn handle_request(
        &mut self,
        ctx: &HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        let (parts, body) = req.into_parts();
        let request_id = get_request_id(ctx, &self.request_ids);
        let url = parts.uri.to_string();
        info!("===== Intercepted Request {}: {} ====", request_id, parts.uri);

        if let Err(e) = request_context_db::store_request_context(
            &self.db_pool,
            &request_id,
            &url
        ) {
            error!("Failed to store request context in DB for {}: {}", request_id, e);
        }

        for (name, value) in &parts.headers {
            trace!("Request {}: Header: {} = {:?}", request_id, name, value);
        }

        let collected_body = match body.collect().await {
            Ok(collected) => collected,
            Err(e) => {
                error!("Request {}: Failed to collect request body: {}", request_id, e);
                return Request::from_parts(parts, Body::empty()).into();
            }
        };

        let body_bytes = collected_body.to_bytes();

        if let Ok(body_str) = std::str::from_utf8(&body_bytes) {
            trace!("Request {}: Body:\n{}", request_id, body_str);
        } else {
            trace!("Request {}: Body: <Non-UTF8 data: {} bytes>", request_id, body_bytes.len());
        }

        // Check for and store linked_tool_use_id
        if let Some(tool_use_id_string) = find_tool_use_id_in_request_body(&body_bytes) {
            if let Err(e) = request_context_db::store_linked_tool_use_id(
                &self.db_pool,
                &request_id,
                &tool_use_id_string
            ) {
                error!("Failed to store linked_tool_use_id in DB for {}: {}", request_id, e);
            }
        }

        Request::from_parts(parts, Body::from(Full::new(body_bytes))).into()
    }

    async fn handle_response(
        &mut self,
        ctx: &HttpContext,
        res: Response<Body>
    ) -> Response<Body> {
        let (parts, body) = res.into_parts();
        let request_id = get_request_id(ctx, &self.request_ids);
        info!("Response {}: Retrieving context from DB...", request_id);
        let (
            request_url,
            linked_tool_use_id
        ) = match request_context_db::retrieve_and_clear_context(&self.db_pool, &request_id) {
            Ok(context) => context,
            Err(e) => {
                error!("Response {}: Failed to retrieve/clear context from DB: {}. Using None.", request_id, e);
                (None, None)
            }
        };
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
                report_url,
                request_id.clone()
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
                report_url,
                request_id.clone()
            ));
            response_body = teed_body;
        }

        // Clear the ID after processing the response for this connection
        clear_request_id(ctx, &self.request_ids);

        info!("Response {}: [{}] Returning teed response to client...", request_id, Utc::now().to_rfc3339());
        Response::from_parts(parts, response_body)
    }
}

impl WebSocketHandler for LogHandler {
    async fn handle_message(&mut self, _ctx: &WebSocketContext, msg: Message) -> Option<Message> {
        trace!("WebSocket message: {:?}", msg);
        Some(msg)
    }
}

fn find_tool_use_id_in_request_body(body_bytes: &[u8]) -> Option<String> {
    // Attempt to parse the body as JSON, return None if fails
    let json_body: Value = serde_json::from_slice(body_bytes).ok()?;
    // Get the messages array, return None if missing or not an array
    let messages = json_body.get("messages")?.as_array()?;
    // Iterate through messages
    for (msg_idx, msg) in messages.iter().enumerate() {
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => debug!("find_tool_use_id: Found message with role 'user' at index {}.", msg_idx),
            _ => continue, // Skip non-user messages
        }
        let content_array = match msg.get("content").and_then(Value::as_array) {
            Some(arr) => arr,
            None => continue, // Skip if no array
        };
        // Iterate through content items
        for item in content_array.iter() {
            match item.get("type").and_then(Value::as_str) {
                Some("tool_result") => {
                    match item.get("tool_use_id").and_then(Value::as_str) {
                        Some(id_str) => return Some(id_str.to_string()), // Found it!
                        None => warn!("find_tool_use_id: 'tool_result' item missing string 'tool_use_id' field."),
                    }
                },
                _ => continue, // Skip non-tool_result items
            }
        }
    }
    None
}

// Generate or retrieve a unique ID for the current connection
fn get_request_id(ctx: &HttpContext, ids_map: &Arc<Mutex<HashMap<SocketAddr, String>>>) -> String {
    let client_addr = ctx.client_addr;
    let mut map = ids_map.lock().expect("Request ID map lock poisoned");
    map.entry(client_addr).or_insert_with(|| create_request_id()).clone()
}

fn create_request_id() -> String {
    format!("request_{}", nanoid::nanoid!())
}

fn clear_request_id(ctx: &HttpContext, ids_map: &Arc<Mutex<HashMap<SocketAddr, String>>>) {
    let client_addr = ctx.client_addr;
    let mut map = ids_map.lock().expect("Request ID map lock poisoned");
    map.remove(&client_addr);
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError + Send + Sync>> {

    tracing_subscriber::fmt()
        .without_time()
        .init();

    info!("Starting LLM proxy...");

    let env_vars = Arc::new(EnvVars::load());
    let db_pool = init_db(env_vars.db_path.as_deref())?;

    let cleanup_pool = db_pool.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 5));
        loop {
            interval.tick().await;
            trace!("Running periodic DB cleanup...");
            if let Err(e) = cleanup_old_contexts(&cleanup_pool, 60 * 10) {
                warn!("Error during DB cleanup: {}", e);
            }
        }
    });

    let signing_key = load_or_generate_signing_key()?;
    let signing_key_arc = Arc::new(signing_key);

    let (ca_key_pair, ca_cert) = load_or_generate_ca()?;

    let ca = RcgenAuthority::new(
        ca_key_pair,
        ca_cert,
        1_000,
        aws_lc_rs::default_provider()
    );

    let env_for_handlers = env_vars.clone();

    // Initialize the shared request ID map
    let request_ids_map = Arc::new(Mutex::new(HashMap::<SocketAddr, String>::new()));

    let proxy = Proxy::builder()
        .with_addr(SocketAddr::from(([0, 0, 0, 0], 8080)))
        .with_ca(ca)
        .with_rustls_client(aws_lc_rs::default_provider())
        .with_http_handler(LogHandler {
            signing_key: signing_key_arc.clone(),
            db_pool: db_pool.clone(),
            env: env_for_handlers.clone(),
            request_ids: request_ids_map.clone(),
        })
        .with_websocket_handler(LogHandler {
            signing_key: signing_key_arc.clone(),
            db_pool: db_pool.clone(),
            env: env_for_handlers.clone(),
            request_ids: request_ids_map.clone(),
        })
        .with_graceful_shutdown(shutdown_signal())
        .build()
        .map_err(|e| format!("Failed to build proxy: {}", e))?;

    let db_mode = if env_vars.db_path.is_some() { "file" } else { "memory" };
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