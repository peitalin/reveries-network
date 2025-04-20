use hudsucker::{
    hyper::{Request, Response},
    rustls::crypto::aws_lc_rs,
    tokio_tungstenite::tungstenite::Message,
    Proxy, HttpHandler, HttpContext, RequestOrResponse, Body, WebSocketHandler, WebSocketContext,
    certificate_authority::RcgenAuthority,
};
use hudsucker::*;
use rcgen::{Certificate, KeyPair, CertificateParams};
use std::{
    net::SocketAddr,
};
use tracing::*;

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
        // Log the request details
        info!("Intercepted request to: {}", req.uri());

        // Log any relevant headers
        for (name, value) in req.headers() {
            debug!("Header: {} = {:?}", name, value);
        }

        // Just pass the request through without modification
        req.into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        // Log the response status
        info!("Intercepted response with status: {}", res.status());

        // Log content-type or other interesting headers
        if let Some(content_type) = res.headers().get("content-type") {
            debug!("Content-Type: {:?}", content_type);
        }

        // Return the response without modification
        res
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