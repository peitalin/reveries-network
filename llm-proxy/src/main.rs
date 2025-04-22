mod utils;
mod tee_body_sse;
mod tee_body;
mod usage;

use hudsucker::{
    hyper::{self, Request, Response},
    rustls::crypto::aws_lc_rs,
    tokio_tungstenite::tungstenite::Message,
    Proxy, HttpHandler, HttpContext, RequestOrResponse, Body, WebSocketHandler, WebSocketContext,
    certificate_authority::RcgenAuthority,
};
use hudsucker::*;
use http_body_util::{Full, BodyExt};
use rcgen::{KeyPair, CertificateParams};
use std::net::SocketAddr;
use tracing::*;
use hyper::header::CONTENT_TYPE;
use chrono::Utc;

use crate::usage::{log_sse_response_task, log_regular_response_task};


async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("Failed to install CTRL+C signal handler: {}", e))
        .expect("Ctrl-C handler setup failed");
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
        let (parts, body) = res.into_parts();
        info!("Intercepted response with status: {}", parts.status);
        let content_type = parts.headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok());
        let is_sse = content_type.map_or(false, |ct| ct.starts_with("text/event-stream"));
        let headers_for_log = parts.headers.clone();
        let response_body;

        if is_sse {
            info!("SSE stream detected, using SSE logging task.");
            let (teed_body, receiver) = tee_body_sse::tee_body_sse(body);
            response_body = teed_body;
            // Spawn the dedicated SSE logging task
            tokio::spawn(log_sse_response_task(receiver, headers_for_log));
        } else {
            info!("Non-SSE response detected, using full body logging task.");
            let (teed_body, receiver) = tee_body::tee_body(body);
            response_body = teed_body;
            // Spawn the dedicated regular logging task
            tokio::spawn(log_regular_response_task(receiver, headers_for_log));
        }

        info!("[{}] Returning teed response to client...", Utc::now().to_rfc3339());
        Response::from_parts(parts, response_body)
    }
}

impl WebSocketHandler for LogHandler {
    async fn handle_message(&mut self, _ctx: &WebSocketContext, msg: Message) -> Option<Message> {
        info!("WebSocket message: {:?}", msg);
        Some(msg)
    }
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
        .map_err(|e| format!("Failed to parse private key from embedded data: {}", e))?;

    // Load the certificate parameters from the PEM string
    let params = CertificateParams::from_ca_cert_pem(ca_cert_pem)
        .map_err(|e| format!("Failed to parse CA certificate from embedded data: {}", e))?;

    // Self-sign the certificate
    let ca_cert = params.self_signed(&key_pair)
        .map_err(|e| format!("Failed to self-sign CA certificate: {}", e))?;

    // Create CA authority using the loaded rcgen key and certificate
    let ca = RcgenAuthority::new(
        key_pair,
        ca_cert,
        1_000, // Cache size for generated certificates
        aws_lc_rs::default_provider()
    ); // RcgenAuthority::new does not return a Result

    // Build and start the proxy
    let proxy = Proxy::builder()
        .with_addr(SocketAddr::from(([0, 0, 0, 0], 8080)))
        .with_ca(ca)
        .with_rustls_client(aws_lc_rs::default_provider())
        .with_http_handler(LogHandler)
        .with_websocket_handler(LogHandler)
        .with_graceful_shutdown(shutdown_signal())
        .build()
        .map_err(|e| format!("Failed to build proxy: {}", e))?; // Map error here

    info!("LLM MITM proxy listening on 0.0.0.0:8080");
    // info!("Clients need to trust the CA cert found in ./ca/hudsucker.cer"); // Comment out if embedded

    proxy.start().await?; // Propagate error from start

    Ok(())
}