mod parser;
mod signers_certs;
mod tee_body_sse;
mod tee_body;
mod usage;

use std::{
    net::SocketAddr,
    error::Error as StdError,
    sync::Arc,
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
use tracing::*;
use p256::ecdsa::SigningKey;

use crate::signers_certs::{load_or_generate_signing_key, load_or_generate_ca};
use crate::usage::{log_sse_response_task, log_regular_response_task};



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

        println!("===== Intercepted Request to: {} =====", parts.uri);
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

        match std::str::from_utf8(&body_bytes) {
            Ok(body_str) => println!("Request Body:\n{}\n", body_str),
            Err(_) => println!("Request Body: <Non-UTF8 data: {} bytes>\n", body_bytes.len()),
        };

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


#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError + Send + Sync>> {
    tracing_subscriber::fmt::init();
    info!("Starting LLM proxy...");

    let signing_key = load_or_generate_signing_key()?;
    let signing_key_arc = Arc::new(signing_key);

    let (ca_key_pair, ca_cert) = load_or_generate_ca()?;

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

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .map_err(|e| format!("Failed to install CTRL+C signal handler: {}", e))
        .expect("Ctrl-C handler setup failed");
}