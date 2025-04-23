use bytes::Bytes;
use chrono::Utc;
use hudsucker::hyper::header::{CONTENT_ENCODING, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;
use tracing::{info, error, warn, debug, trace};
use std::sync::Arc; // For Arc<SigningKey>
use std::error::Error as StdError;

// Crypto imports
use p256::ecdsa::SigningKey;
use p256::ecdsa::signature::Signer; // Import the Signer trait
use p256::ecdsa::Signature; // Use this concrete type
use base64::{Engine as _, engine::general_purpose::STANDARD as base64_standard}; // For encoding signature

use crate::utils;
use crate::tee_body_sse::SSEUpdate;



#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UsageData {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

impl UsageData {
    pub fn new() -> Self {
        UsageData {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    }
}

#[derive(Serialize, Debug)] // Only needs Serialize to send
struct SignedUsageReport {
    payload: String, // Base64 encoded JSON of UsageData + Timestamp
    signature: String, // Base64 encoded ECDSA signature
}

#[derive(Serialize, Debug)] // Inner payload for signing
struct UsageReportPayload {
    usage: UsageData,
    timestamp: i64, // Unix timestamp
    // Add nonce later if needed for replay protection
}

/// Sends the signed usage report to the p2p-node.
async fn submit_usage_report(report: SignedUsageReport) {
    // TODO: Get target URL from config/env var
    let target_url = "http://localhost:9999/report_usage"; // Placeholder

    info!("Submitting usage report to: {}", target_url);

    // Use a static reqwest client or create one per call
    // Consider connection pooling for performance
    let client = reqwest::Client::new();

    match client.post(target_url)
        .json(&report)
        .send()
        .await
    {
        Ok(response) => {
            if response.status().is_success() {
                info!("Successfully submitted usage report.");
            } else {
                error!(
                    "Failed to submit usage report. Status: {}. Body: {:?}",
                    response.status(),
                    response.text().await.unwrap_or_else(|_| "<failed to read body>".to_string())
                );
            }
        },
        Err(e) => {
            error!("Error sending usage report request: {}", e);
        }
    }
}

/// Processes the aggregated body of a non-SSE response.
fn process_and_log_regular_body(
    log_buffer: Vec<u8>,
    headers: &HeaderMap<HeaderValue>,
    signing_key: &Arc<SigningKey> // Pass signing key
) {
    println!("--- Background Log: Response ---");
    let content_encoding = headers.get(CONTENT_ENCODING).and_then(|h| h.to_str().ok());

    match utils::decompress_body(content_encoding, &Bytes::from(log_buffer)) {
        Ok(decompressed_bytes) => {
            match utils::parse_json_and_extract_usage(&decompressed_bytes) {
                Ok((json_value, usage_option)) => {
                    // Log JSON
                    match serde_json::to_string_pretty(&json_value) {
                        Ok(pretty_json) => println!("Body (JSON): {} ", pretty_json),
                        Err(_) => println!("Body (Raw JSON): {:?}", json_value),
                    }
                    // Sign and submit usage if extracted
                    if let Some(usage_data) = usage_option {
                        let payload = UsageReportPayload {
                            usage: usage_data.clone(), // Clone usage data
                            timestamp: Utc::now().timestamp(),
                        };
                        match serde_json::to_vec(&payload) { // Serialize to bytes
                            Ok(payload_bytes) => {
                                let signature: Signature = signing_key.sign(&payload_bytes);
                                let signed_report = SignedUsageReport {
                                    payload: base64_standard.encode(&payload_bytes),
                                    signature: base64_standard.encode(signature.to_bytes()),
                                };
                                // Spawn task to send the report
                                tokio::spawn(submit_usage_report(signed_report));
                            },
                            Err(e) => error!("Failed to serialize usage payload for signing: {}", e),
                        }
                    }
                },
                Err(e) => { // ProcessError
                    warn!("Failed to parse response body as JSON: {}. Falling back to string.", e);
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


/// Background task for non-SSE responses.
pub async fn log_regular_response_task(
    mut receiver: Receiver<Result<Bytes, Box<dyn StdError + Send + Sync>>>,
    headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>, // Accept Arc<SigningKey>
) {
    let mut log_buffer = Vec::new();
    let mut stream_error = false;

    while let Some(result) = receiver.recv().await {
        match result {
            Ok(chunk) => log_buffer.extend_from_slice(&chunk),
            Err(e) => {
                error!("Error received from TeeBody channel: {}", e);
                stream_error = true;
                break;
            }
        }
    }

    if !stream_error {
        info!("Logging task finished receiving stream ({} bytes)", log_buffer.len());
        if !log_buffer.is_empty() {
            process_and_log_regular_body(log_buffer, &headers, &signing_key);
        }
    } else {
        warn!("Logging task stopped due to stream error. Logged {} bytes before error.", log_buffer.len());
    }
    info!("[{}] Background log task finished.", Utc::now().to_rfc3339());
}

/// Background task for SSE responses.
pub async fn log_sse_response_task(
    mut receiver: Receiver<SSEUpdate>,
    _headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>, // Accept Arc<SigningKey>
) {
    let mut current_usage = UsageData::new();
    let mut final_usage_to_submit = UsageData::default();

    println!("--- Background Log: SSE Events Start ---");
    while let Some(update) = receiver.recv().await {
        debug!("Received SSE Update: {:?}", update);
        match update {
            SSEUpdate::UsageStart { input_tokens } => {
                current_usage.input_tokens = input_tokens;
            },
            SSEUpdate::UsageDelta { output_tokens } => {
                current_usage.output_tokens = output_tokens;
            },
            SSEUpdate::Stop => {
                if current_usage.input_tokens > 0 || current_usage.output_tokens > 0 {
                    final_usage_to_submit = current_usage.clone(); // Capture usage to sign
                    // --- Sign and Submit ---
                     let payload = UsageReportPayload {
                         usage: final_usage_to_submit.clone(), // Clone usage data
                         timestamp: Utc::now().timestamp(),
                     };
                     match serde_json::to_vec(&payload) { // Serialize to bytes
                         Ok(payload_bytes) => {
                             let signature: Signature = signing_key.sign(&payload_bytes);
                             let signed_report = SignedUsageReport {
                                 payload: base64_standard.encode(&payload_bytes),
                                 signature: base64_standard.encode(signature.to_bytes()),
                             };
                             // Spawn task to send the report
                             tokio::spawn(submit_usage_report(signed_report));
                         },
                         Err(e) => error!("Failed to serialize SSE usage payload for signing: {}", e),
                     }
                     // -------------------
                    current_usage = UsageData::new(); // Reset
                }
            },
            SSEUpdate::Text(text) => {
                trace!("Received SSE Text: {}", text);
            },
            SSEUpdate::Other(s) => {
                debug!("Received SSE Other: {}", s);
            },
        }
    }
    println!("--- SSE Event Stream Ended ---");
    info!("[{}] Background SSE log task finished.", Utc::now().to_rfc3339());
    // Log the usage that was actually submitted
    info!("Final SSE Token Usage Submitted: {:?}", final_usage_to_submit);
}