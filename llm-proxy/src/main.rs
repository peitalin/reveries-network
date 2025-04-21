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
use rcgen::{Certificate, KeyPair, CertificateParams};
use std::net::SocketAddr;
use tracing::*;
use std::io::Read;
use serde_json::Value;
use hyper::header::CONTENT_ENCODING;

// Add the utils module
mod utils;

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

        // Log content-type or other interesting headers
        if let Some(content_type) = parts.headers.get("content-type") {
            info!("Content-Type: {:?}", content_type);
        }

        // Aggregate the body stream using HttpBodyExt::collect()
        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(e) => {
                error!("Failed to collect response body: {}", e);
                // Reconstruct response with empty body on error
                return Response::from_parts(parts, Body::empty());
            }
        };

        println!("\n=========== Response ===========");
        // Check content encoding and decompress if necessary
        let content_encoding = parts.headers.get(CONTENT_ENCODING)
            .and_then(|h| h.to_str().ok());

        // Call the utility function to decompress the body
        match utils::decompress_body(content_encoding, &body_bytes) {
            Ok(decompressed_bytes) => {
                // Attempt to log the decompressed body as JSON or fallback to string
                match serde_json::from_slice::<Value>(&decompressed_bytes) {
                    Ok(json_value) => {
                        // Pretty print the JSON
                        match serde_json::to_string_pretty(&json_value) {
                            Ok(pretty_json) => println!("Response Body (JSON):
{}
", pretty_json),
                            Err(_) => println!("Response Body (Raw JSON): {:?}", json_value), // Fallback
                        }
                    },
                    Err(_) => {
                        // If JSON parsing fails, try logging as UTF-8 string
                        match std::str::from_utf8(&decompressed_bytes) {
                            Ok(body_str) => println!("Response Body (String):
{}
", body_str),
                            Err(_) => println!("Response Body: <Non-UTF8 data: {} bytes>", decompressed_bytes.len()),
                        }
                    }
                }
            },
            Err(e) => {
                // Log decompression error
                error!("Failed to decompress response body: {}", e);
                println!("Response Body: <Decompression Failed - Original {} bytes>", body_bytes.len());
            }
        }

        println!("================================");

        // IMPORTANT: Reconstruct the response with the ORIGINAL body bytes (potentially compressed)
        Response::from_parts(parts, Body::from(Full::new(body_bytes)))
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