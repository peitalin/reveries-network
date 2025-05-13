use std::sync::Arc;
use bytes::Bytes;
use chrono::Utc;
use color_eyre::eyre::{anyhow, Result};
use hudsucker::hyper::header::{CONTENT_ENCODING, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;
use tracing::{info, error, warn, debug, trace};
use p256::ecdsa::{
    SigningKey,
    signature::Signer,
    Signature,
};
use base64::{
    Engine,
    engine::general_purpose::STANDARD as base64_standard // For encoding signature
};
use once_cell::sync::Lazy;

use runtime::tee_attestation::{
    generate_tee_attestation_with_data,
    hash_payload_for_tdx_report_data,
};
use crate::parser;
use crate::tee_body::ChannelError;

// Global static reqwest client with connection pooling
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .pool_max_idle_per_host(5) // Keep up to 5 idle connections per host
        .pool_idle_timeout(std::time::Duration::from_secs(30)) // Keep connections alive for 30 seconds
        .timeout(std::time::Duration::from_secs(10)) // Set request timeout to 10 seconds
        .build()
        .expect("Failed to create reqwest HTTP client")
});

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct UsageData {
    pub reverie_id: Option<String>,
    pub spender: Option<String>,
    pub spender_type: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub tool_use: Option<ToolUse>,
}

impl UsageData {
    pub fn new() -> Self {
        UsageData {
            reverie_id: None,
            spender: None,
            spender_type: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            tool_use: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolUse {
    pub id: String,               // The tool_use ID from the response
    pub name: String,             // The name of the tool that was used
    pub input: serde_json::Value, // The input provided to the tool (as a JSON object)
    pub tool_type: String,        // The type of the tool (usually "tool_use")
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SignedUsageReport {
    pub payload: String,      // Base64 encoded JSON of UsageData + Timestamp
    pub signature: String,    // Base64 encoded ECDSA signature
    pub tdx_quote: String,    // Base64 encoded TDX QuoteV4 (attestation)
}

#[derive(Debug, Deserialize, Serialize)] // Inner payload for signing
pub struct UsageReportPayload {
    pub usage: UsageData,
    pub timestamp: i64, // Unix timestamp
    pub linked_tool_use_id: Option<String>,
    pub request_id: String,
}

#[derive(Serialize, Debug)]
struct JsonRpcRequest<'a, T> {
    jsonrpc: &'a str,
    method: &'a str,
    params: T,
    id: u64,
}

/// Sends the signed usage report to the p2p-node.
async fn submit_usage_report(report: SignedUsageReport, target_url: String) {

    let rpc_request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "report_usage",
        params: report,
        id: 1,
    };

    info!("Submitting JSON-RPC usage report to: {}", target_url);

    match HTTP_CLIENT.post(&target_url)
        .json(&rpc_request)
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

fn process_and_log_regular_body(
    log_buffer: Vec<u8>,
    headers: &HeaderMap<HeaderValue>,
    signing_key: &Arc<SigningKey>,
    request_url: Option<String>,
    linked_tool_use_id: Option<String>,
    reverie_id: Option<String>,
    report_target_url: String,
    request_id: String,
    spender: Option<String>,
    spender_type: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {

    let content_encoding = headers.get(CONTENT_ENCODING).and_then(|h| h.to_str().ok());
    let decompressed_bytes = parser::decompress_body(
        content_encoding,
        &Bytes::from(log_buffer)
    )?;

    let mut usage_report_payload = match parser::parse_json_and_extract_usage(
        &decompressed_bytes,
        request_url.as_deref()
    ) {
        Err(e) => {
            warn!("Failed to parse response body as JSON: {}. Falling back to string.", e);
            if let Ok(body_str) = std::str::from_utf8(&decompressed_bytes) {
                info!("Body (String): {} ", body_str);
            } else {
                info!("Body: <Non-UTF8 data: {} bytes>", decompressed_bytes.len());
            }
            Err(Box::new(e))
        }
        Ok((json_value, usage_option)) => {
            match serde_json::to_string_pretty(&json_value) {
                Ok(pretty_json) => info!("Body (JSON): {} ", pretty_json),
                Err(_) => info!("Body (Raw JSON): {:?}", json_value),
            };
            let mut usage_data = match usage_option {
                None => return Err(anyhow!("No usage data found in response.").into()),
                Some(usage_data) => {
                    if let Some(ref tool_use) = usage_data.tool_use {
                        info!("Parser Check: Tool use detected in response: {} (id: {})", tool_use.name, tool_use.id);
                    }
                    usage_data
                },
            };
            usage_data.reverie_id = reverie_id.clone();
            Ok(UsageReportPayload {
                usage: usage_data,
                timestamp: Utc::now().timestamp(),
                linked_tool_use_id: linked_tool_use_id.clone(),
                request_id: request_id.clone(),
            })
        }
    }?;

    usage_report_payload.usage.spender = spender.clone();
    usage_report_payload.usage.spender_type = spender_type.clone();

    // Sign and submit
    match serde_json::to_vec(&usage_report_payload) {
        Ok(payload_bytes) => {
            let signature: Signature = signing_key.sign(&payload_bytes);
            let report_data = hash_payload_for_tdx_report_data(&payload_bytes);

            let (_quote, quote_bytes) = generate_tee_attestation_with_data(report_data, false)
                .expect("TEE attestation generation error");
            let tdx_quote = base64_standard.encode(&quote_bytes);
            let signed_report = SignedUsageReport {
                payload: base64_standard.encode(&payload_bytes),
                signature: base64_standard.encode(signature.to_bytes()),
                tdx_quote,
            };
            // Pass the URL to the submission task
            tokio::spawn(submit_usage_report(signed_report, report_target_url.clone()));
            Ok(())
        },
        Err(e) => {
            error!("Failed to serialize usage payload for signing: {}", e);
            Err(e.into())
        }
    }
}

/// Background task for non-SSE responses.
pub async fn log_regular_response_task(
    mut receiver: Receiver<Result<Bytes, ChannelError>>,
    headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>,
    request_url: Option<String>,
    linked_tool_use_id: Option<String>,
    reverie_id: Option<String>,
    report_target_url: String,
    request_id: String,
    spender: Option<String>,
    spender_type: Option<String>,
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
            if let Err(e) = process_and_log_regular_body(
                log_buffer,
                &headers,
                &signing_key,
                request_url,
                linked_tool_use_id,
                reverie_id,
                report_target_url,
                request_id.clone(),
                spender,
                spender_type,
            ) {
                error!("Error processing regular body for request {}: {}", request_id, e);
            }
        }
    } else {
        warn!("Logging task stopped due to stream error. Logged {} bytes before error.", log_buffer.len());
    }
    info!("[{}] Background log task finished for request {}.", Utc::now().to_rfc3339(), request_id);
}

/// Background task for SSE responses.
pub async fn log_sse_response_task(
    mut receiver: Receiver<parser::SSEChunk>,
    _headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>,
    request_url: Option<String>,
    linked_tool_use_id: Option<String>,
    reverie_id: Option<String>,
    report_target_url: String,
    request_id: String,
    spender: Option<String>,
    spender_type: Option<String>,
) {
    let mut current_usage = UsageData::new();
    current_usage.reverie_id = reverie_id.clone();
    let mut final_usage_to_submit = UsageData::default();
    final_usage_to_submit.reverie_id = reverie_id.clone();
    final_usage_to_submit.spender = spender.clone();
    final_usage_to_submit.spender_type = spender_type.clone();

    println!("--- Background Log: SSE Events Start ---");
    if let Some(url) = &request_url {
        println!("Processing SSE events from URL: {}", url);
    }

    while let Some(update) = receiver.recv().await {
        debug!("Received SSE Update: {:?}", update);
        match update {
            parser::SSEChunk::InputTokens { input_tokens } => {
                current_usage.input_tokens = input_tokens;
            },
            parser::SSEChunk::OutputTokens { output_tokens } => {
                current_usage.output_tokens = output_tokens;
            },
            parser::SSEChunk::ToolUse { id, name, input, tool_type } => {
                // Update usage data with tool use information
                info!("Tool use detected: {} ({})", name, id);
                current_usage.tool_use = Some(ToolUse {
                    id,
                    name,
                    input,
                    tool_type,
                });
            },
            parser::SSEChunk::Stop => {
                if current_usage.input_tokens > 0 || current_usage.output_tokens > 0 {
                    // Capture usage to sign
                    final_usage_to_submit = current_usage.clone();
                    // Sign and Submit
                    let payload = UsageReportPayload {
                        usage: final_usage_to_submit.clone(),
                        timestamp: Utc::now().timestamp(),
                        linked_tool_use_id: linked_tool_use_id.clone(),
                        request_id: request_id.clone(),
                    };
                    // Log tool usage information if present
                    if let Some(ref tool_use) = final_usage_to_submit.tool_use {
                        info!(
                            "Final SSE Usage includes tool use: {} (id: {})",
                            tool_use.name,
                            tool_use.id
                        );
                    }

                    match serde_json::to_vec(&payload) {
                        Ok(payload_bytes) => {
                            let signature: Signature = signing_key.sign(&payload_bytes);
                            let report_data = runtime::tee_attestation::hash_payload_for_tdx_report_data(&payload_bytes);

                            let (_quote, quote_bytes) = generate_tee_attestation_with_data(report_data, false)
                                .expect("TEE attestation generation error");
                            let tdx_quote = base64_standard.encode(&quote_bytes);
                            let signed_report = SignedUsageReport {
                                payload: base64_standard.encode(&payload_bytes),
                                signature: base64_standard.encode(signature.to_bytes()),
                                tdx_quote,
                            };
                            tokio::spawn(submit_usage_report(signed_report, report_target_url.clone()));
                        },
                        Err(e) => error!("Failed to serialize SSE usage payload for signing: {}", e),
                    }
                    current_usage = UsageData::new();
                    current_usage.reverie_id = reverie_id.clone();
                }
            },
            parser::SSEChunk::Text(text) => {
                trace!("Received SSE Text: {}", text);
            },
            parser::SSEChunk::Other(s) => {
                debug!("Received SSE Other: {}", s);
            },
        }
    }
    println!("--- SSE Event Stream Ended ---");
    info!("[{}] Background SSE log task finished for request {}.", Utc::now().to_rfc3339(), request_id);
    info!("Final SSE Token Usage Submitted for request {}: {:?}", request_id, final_usage_to_submit);
}
