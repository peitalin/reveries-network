use hudsucker::{
    hyper::{self, Request, Response, body::Bytes},
    rustls::crypto::aws_lc_rs,
    tokio_tungstenite::tungstenite::Message,
    Proxy, HttpHandler, HttpContext, RequestOrResponse, Body, WebSocketHandler, WebSocketContext,
    certificate_authority::RcgenAuthority,
};
use http_body_util::BodyExt;
use http_body_util::Full;
use hudsucker::*;
use rcgen::{KeyPair, CertificateParams};
use std::net::SocketAddr;
use tracing::*;
use serde_json::Value;
use hyper::header::{HeaderValue, CONTENT_ENCODING, CONTENT_TYPE};
use hyper::HeaderMap;
use tokio::sync::mpsc::Receiver;
use chrono::Utc;

mod utils;
mod tee_body_sse;
mod tee_body;

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C signal handler");
}

#[derive(Clone)]
struct LogHandler;

impl HttpHandler for LogHandler {
    async fn handle_request(
        &mut self,
        _ctx: &HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        // Split the request into parts to access the body
        let (parts, body) = req.into_parts();

        // Log the request details
        info!("Intercepted request to: {}", parts.uri);

        // Log any relevant headers
        for (name, value) in &parts.headers {
            info!("Header: {} = {:?}", name, value);
        }

        // Aggregate the body stream using HttpBodyExt::collect()
        let collected_body = match body.collect().await {
            Ok(collected) => collected,
            Err(e) => {
                error!("Failed to collect request body: {}", e);
                // Reconstruct request with empty body on error and return early
                return Request::from_parts(parts, Body::empty()).into();
            }
        };

        // Get the body bytes from the collected data
        let body_bytes = collected_body.to_bytes();

        println!("\n=========== Request ============");
        // Attempt to log the body as UTF-8 string
        match std::str::from_utf8(&body_bytes) {
            Ok(body_str) => println!("Request Body:\n{}\n", body_str),
            Err(_) => println!("Request Body: <Non-UTF8 data: {} bytes>\n", body_bytes.len()),
        };
        println!("================================");

        // Reconstruct the request with the collected body bytes using Full::new()
        Request::from_parts(parts, Body::from(Full::new(body_bytes))).into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        // Split the response into parts to access the body
        let (parts, body) = res.into_parts();

        // Log the response status
        info!("Intercepted response with status: {}", parts.status);

        // Check Content-Type to determine logging strategy
        let content_type = parts.headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok());
        let is_sse = content_type.map_or(false, |ct| ct.starts_with("text/event-stream"));

        // Clone headers needed for the logging task
        let headers_for_log = parts.headers.clone();

        let response_body;

        if is_sse {
            info!("SSE stream detected, using SSE logging task.");
            // Create the SSE teed body and receiver
            let (teed_body, mut receiver) = tee_body_sse::tee_body_sse(body);
            response_body = teed_body; // Assign the teed body to be returned

            // Spawn a background task specifically for logging SSE events
            tokio::spawn(async move {
                println!("--- Background Log: SSE Events ---");
                while let Some(event_str) = receiver.recv().await {
                    // Simply print the received event string
                    print!("SSE Event:\n{}", event_str); // Print directly as it includes newlines
                }
                println!("--- SSE Event Stream Ended ---");
                info!("[{}] Background SSE log task finished.", Utc::now().to_rfc3339());
            });

        } else {
            info!("Non-SSE response detected, using full body logging task.");
            // Create the regular teed body and receiver
            let (teed_body, receiver) = tee_body::tee_body(body);
            response_body = teed_body; // Assign the teed body to be returned

            // Spawn a background task to handle logging by calling the dedicated function
            tokio::spawn(log_response_body_task(receiver, headers_for_log));
        }

        // Return the response immediately with the teed body
        info!("[{}] Returning teed response to client...", Utc::now().to_rfc3339());
        Response::from_parts(parts, response_body) // Return the chosen body type
    }
}

impl WebSocketHandler for LogHandler {
    async fn handle_message(&mut self, _ctx: &WebSocketContext, msg: Message) -> Option<Message> {
        info!("WebSocket message: {:?}", msg);
        Some(msg)
    }
}

/// Background task to aggregate, decompress, and log the response body.
async fn log_response_body_task(
    mut receiver: Receiver<Result<Bytes, Box<dyn std::error::Error + Send + Sync>>>,
    headers: HeaderMap<HeaderValue>,
) {
    let mut log_buffer = Vec::new();
    let mut stream_error = None;

    // Aggregate chunks from the receiver
    while let Some(result) = receiver.recv().await {
        match result {
            Ok(chunk) => log_buffer.extend_from_slice(&chunk),
            Err(e) => {
                error!("Error received from TeeBody channel: {}", e);
                stream_error = Some(e); // Store the error
                break; // Stop processing on error
            }
        }
    }

    if stream_error.is_none() {
        info!("Logging task finished receiving stream ({} bytes)", log_buffer.len());
    } else {
        warn!("Logging task stopped due to stream error. Logged {} bytes before error.", log_buffer.len());
    }

    // Proceed with logging only if the buffer is not empty
    if !log_buffer.is_empty() {
        println!("--- Background Log: Response ---");
        let content_encoding = headers.get(CONTENT_ENCODING).and_then(|h| h.to_str().ok());

        // Decompress the aggregated body
        match utils::decompress_body(content_encoding, &Bytes::from(log_buffer)) { // Convert Vec<u8> to Bytes
            Ok(decompressed_bytes) => {
                // Attempt to log as JSON or fallback
                match serde_json::from_slice::<Value>(&decompressed_bytes) {
                    Ok(json_value) => {
                        match serde_json::to_string_pretty(&json_value) {
                            Ok(pretty_json) => println!("Body (JSON): {} ", pretty_json),
                            Err(_) => println!("Body (Raw JSON): {:?}", json_value),
                        }
                    },
                    Err(_) => {
                        match std::str::from_utf8(&decompressed_bytes) {
                            Ok(body_str) => println!("Body (String): {} ", body_str),
                            Err(_) => println!("Body: <Non-UTF8 data: {} bytes>", decompressed_bytes.len()),
                        }
                    }
                }
            },
            Err(e) => {
                error!("Log: Failed to decompress body: {}", e);
                println!("Body: <Decompression Failed>");
            }
        }
         println!("------------------------------");
    }
    // Log when the task finishes
    info!("[{}] Background log task finished.", Utc::now().to_rfc3339());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    info!("Starting LLM proxy...");

    // Load CA cert and key directly into strings using include_str!
    // Paths are relative to the src/main.rs file
    let ca_cert_pem = include_str!("../certs/hudsucker.cer");
    let ca_key_pem = include_str!("../certs/hudsucker.key");

    info!("Loading CA certificate and key from embedded strings...");

    // Load the key pair from the PEM string using rcgen
    let key_pair = KeyPair::from_pem(ca_key_pem)
        .expect("Failed to parse private key from ca/hudsucker.key");
    // info!(">>> Key PEM: {:?}", key_pair); // Removed diagnostic print

    // Load the certificate parameters from the PEM string and self-sign
    let ca_cert = CertificateParams::from_ca_cert_pem(ca_cert_pem)
        .expect("Failed to parse CA certificate from ca/hudsucker.cer")
        .self_signed(&key_pair)
        .expect("Failed to self-sign CA certificate");

    // Create CA authority using the loaded rcgen key and certificate
    let ca = RcgenAuthority::new(
        key_pair,
        ca_cert,
        1_000, // Cache size for generated certificates
        aws_lc_rs::default_provider()
    );

    // Build and start the proxy with the RustTls client configured with system roots
    let proxy = Proxy::builder()
        .with_addr(SocketAddr::from(([0, 0, 0, 0], 8080)))  // Listen on all interfaces
        .with_ca(ca)
        .with_rustls_client(aws_lc_rs::default_provider())  // Use system root certificates
        .with_http_handler(LogHandler)
        .with_websocket_handler(LogHandler)
        .with_graceful_shutdown(shutdown_signal())
        .build()
        .expect("Failed to build proxy");

    info!("LLM MITM proxy listening on 0.0.0.0:8080 using embedded CA from ./ca/");
    info!("Clients need to trust the CA cert found in ./ca/hudsucker.cer");

    proxy.start().await?; // Keep ? here as proxy.start() returns specific Error

    Ok(())
}