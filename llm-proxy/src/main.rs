use hudsucker::{
    certificate_authority::RcgenAuthority,
    hyper::{Request, Response},
    rcgen::{CertifiedKey, CertificateParams, KeyPair},
    rustls::crypto::aws_lc_rs,
    tokio_tungstenite::tungstenite::Message,
    *,
};
use std::{
    fs,
    net::SocketAddr,
    path::Path,
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
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt::init();
    info!("Starting LLM proxy...");

    // Create certs directory
    let certs_dir = Path::new("certs");
    fs::create_dir_all(certs_dir)?;

    // Generate CA certificate for various LLM API domains
    info!("Generating CA certificate...");

    let certified_key = rcgen::generate_simple_self_signed(
        vec![
            "*.anthropic.com".to_string(),
            "*.openai.com".to_string(),
            "*.googleapis.com".to_string(),
            "api.anthropic.com".to_string(),
            "api.openai.com".to_string(),
            "generativelanguage.googleapis.com".to_string(),
            "localhost".to_string()
        ]
    )?;

    // Save the certificate to certs directory
    let cert_path = certs_dir.join("ca.cer");
    fs::write(&cert_path, &certified_key.cert.pem())?;

    info!("CA certificate saved to {}", cert_path.display());

    // Create CA authority for MITM
    let ca = RcgenAuthority::new(
        certified_key.key_pair,
        certified_key.cert,
        1_000,
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
        .build()?;

    info!("LLM proxy listening on 0.0.0.0:8080");
    info!("Certificate available at: {}", cert_path.display());
    info!("Be sure to set REQUESTS_CA_BUNDLE to point to this certificate");

    proxy.start().await?;

    Ok(())
}