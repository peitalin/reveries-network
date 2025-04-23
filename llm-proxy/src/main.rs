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
use std::error::Error as StdError;
use std::path::Path;
use std::sync::Arc;
use std::fs;
use p256::ecdsa::{SigningKey, VerifyingKey};
use ecdsa::elliptic_curve::pkcs8::{EncodePrivateKey, DecodePrivateKey, EncodePublicKey, DecodePublicKey, LineEnding};
use rand_core::OsRng;
use std::io::{Write, Read};

use crate::usage::{log_sse_response_task, log_regular_response_task, UsageData};

const CA_CERT_PATH: &str = "/certs_src/hudsucker.cer";
const CA_KEY_PATH: &str = "/certs_src/hudsucker.key";
const PRIVATE_KEY_PATH: &str = "/data/proxy_private.key";
const PUBLIC_KEY_PATH: &str = "/shared_identity/proxy_public.pem";

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("Failed to install CTRL+C signal handler: {}", e))
        .expect("Ctrl-C handler setup failed");
}

#[derive(Clone)]
struct LogHandler {
    signing_key: Arc<SigningKey>,
}

impl HttpHandler for LogHandler {
    async fn handle_request(
        &mut self,
        _ctx: &HttpContext,
        req: Request<Body>,
    ) -> RequestOrResponse {
        let (parts, body) = req.into_parts();

        info!("Intercepted request to: {}", parts.uri);

        for (name, value) in &parts.headers {
            info!("Header: {} = {:?}", name, value);
        }

        let collected_body = match body.collect().await {
            Ok(collected) => collected,
            Err(e) => {
                error!("Failed to collect request body: {}", e);
                return Request::from_parts(parts, Body::empty()).into();
            }
        };

        let body_bytes = collected_body.to_bytes();

        println!("\n=========== Request ============");
        match std::str::from_utf8(&body_bytes) {
            Ok(body_str) => println!("Request Body:\n{}\n", body_str),
            Err(_) => println!("Request Body: <Non-UTF8 data: {} bytes>\n", body_bytes.len()),
        };
        println!("================================");

        Request::from_parts(parts, Body::from(Full::new(body_bytes))).into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        let (parts, body) = res.into_parts();
        info!("Intercepted response with status: {}", parts.status);
        let content_type = parts.headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok());
        let is_sse = content_type.map_or(false, |ct| ct.starts_with("text/event-stream"));
        let headers_for_log = parts.headers.clone();
        let response_body;
        let key_arc = self.signing_key.clone();

        if is_sse {
            info!("SSE stream detected, using SSE logging task.");
            let (teed_body, receiver) = tee_body_sse::tee_body_sse(body);
            response_body = teed_body;
            tokio::spawn(log_sse_response_task(receiver, headers_for_log, key_arc));
        } else {
            info!("Non-SSE response detected, using full body logging task.");
            let (teed_body, receiver) = tee_body::tee_body(body);
            response_body = teed_body;
            tokio::spawn(log_regular_response_task(receiver, headers_for_log, key_arc));
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

fn load_or_generate_signing_key() -> Result<SigningKey, Box<dyn StdError + Send + Sync>> {
    let private_key_path = Path::new(PRIVATE_KEY_PATH);
    let public_key_path = Path::new(PUBLIC_KEY_PATH);

    // if private_key_path.exists() {
        // info!("Loading existing private key from: {}", PRIVATE_KEY_PATH);
        // let pem_str = fs::read_to_string(private_key_path)
        //     .map_err(|e| format!("Failed to read private key file: {}", e))?;
        // let key = SigningKey::from_pkcs8_pem(&pem_str)
        //     .map_err(|e| format!("Failed to parse private key PKCS8 PEM: {}", e))?;

        // if public_key_path.exists() {
        //     match fs::read_to_string(public_key_path) {
        //         Ok(pub_pem_str) => {
        //             match VerifyingKey::from_public_key_pem(&pub_pem_str) {
        //                 Ok(public_key) => {
        //                     if key.verifying_key() != &public_key {
        //                         warn!("Private key does not match existing public key! Consider deleting keys and regenerating.");
        //                     }
        //                 },
        //                 Err(e) => warn!("Failed to parse existing public key PEM for verification: {}", e),
        //             }
        //         },
        //         Err(e) => warn!("Failed to read existing public key PEM for verification: {}", e),
        //     }
        // }
        // Ok(key)

        info!("Generating new ECDSA P-256 key pair.");
        let signing_key = SigningKey::random(&mut OsRng);

        if let Some(parent) = private_key_path.parent() { fs::create_dir_all(parent)?; }
        if let Some(parent) = public_key_path.parent() { fs::create_dir_all(parent)?; }

        let private_key_pem = signing_key.to_pkcs8_pem(LineEnding::LF)
            .map_err(|e| format!("Failed to encode private key to PKCS8 PEM: {}", e))?;
        fs::write(private_key_path, private_key_pem.as_bytes())
            .map_err(|e| format!("Failed to write private key: {}", e))?;
        info!("Saved new private key to: {}", PRIVATE_KEY_PATH);

        let public_key_pem = signing_key.verifying_key().to_public_key_pem(LineEnding::LF)
             .map_err(|e| format!("Failed to encode public key to PKCS8 PEM: {}", e))?;
        fs::write(public_key_path, public_key_pem.as_bytes())
            .map_err(|e| format!("Failed to write public key: {}", e))?;
        info!("Saved new public key to: {}", PUBLIC_KEY_PATH);

        Ok(signing_key)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError + Send + Sync>> {
    tracing_subscriber::fmt::init();
    info!("Starting LLM proxy...");

    let signing_key = load_or_generate_signing_key()?;
    let signing_key_arc = Arc::new(signing_key);

    info!("Loading CA certificate and key from files: {}, {}", CA_CERT_PATH, CA_KEY_PATH);
    let ca_cert_pem = fs::read_to_string(CA_CERT_PATH)
        .map_err(|e| format!("Failed to read CA cert file {}: {}", CA_CERT_PATH, e))?;
    let ca_key_pem = fs::read_to_string(CA_KEY_PATH)
        .map_err(|e| format!("Failed to read CA key file {}: {}", CA_KEY_PATH, e))?;
    let ca_key_pair = KeyPair::from_pem(&ca_key_pem)
        .map_err(|e| format!("Failed to parse CA private key: {}", e))?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
        .map_err(|e| format!("Failed to parse CA certificate: {}", e))?;
    let ca_cert = ca_params.self_signed(&ca_key_pair)
        .map_err(|e| format!("Failed to self-sign CA certificate: {}", e))?;
    let ca = RcgenAuthority::new(
        ca_key_pair,
        ca_cert,
        1_000,
        aws_lc_rs::default_provider()
    );

    let key_ref_for_http = signing_key_arc.clone();
    let key_ref_for_ws = signing_key_arc.clone();

    let proxy = Proxy::builder()
        .with_addr(SocketAddr::from(([0, 0, 0, 0], 8080)))
        .with_ca(ca)
        .with_rustls_client(aws_lc_rs::default_provider())
        .with_http_handler(LogHandler { signing_key: key_ref_for_http })
        .with_websocket_handler(LogHandler { signing_key: key_ref_for_ws })
        .with_graceful_shutdown(shutdown_signal())
        .build()
        .map_err(|e| format!("Failed to build proxy: {}", e))?;

    info!("llm-proxy listening on 0.0.0.0:8080");
    proxy.start().await?;
    Ok(())
}