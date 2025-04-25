use std::sync::Arc;
use std::env;
use bytes::Bytes;
use chrono::Utc;
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
use crate::parser;
use crate::tee_body::ChannelError;

// Define a global static reqwest client with connection pooling
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
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    // Tool usage tracking in a single field
    pub tool_use: Option<ToolUse>,
}

impl UsageData {
    pub fn new() -> Self {
        UsageData {
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
    pub id: String,              // The tool_use ID from the response
    pub name: String,            // The name of the tool that was used
    pub input: serde_json::Value, // The input provided to the tool (as a JSON object)
    pub tool_type: String,       // The type of the tool (usually "tool_use")
}

#[derive(Debug, Deserialize, Serialize)] // Only needs Serialize to send
pub struct SignedUsageReport {
    pub payload: String, // Base64 encoded JSON of UsageData + Timestamp
    pub signature: String, // Base64 encoded ECDSA signature
}

#[derive(Debug, Deserialize, Serialize)] // Inner payload for signing
pub struct UsageReportPayload {
    pub usage: UsageData,
    pub timestamp: i64, // Unix timestamp
    // Add nonce later if needed for replay protection
}

#[derive(Serialize, Debug)]
struct JsonRpcRequest<'a, T> {
    jsonrpc: &'a str,
    method: &'a str,
    params: T, // Generic parameter type
    id: u64, // Using a simple numeric ID for now
}

/// Sends the signed usage report to the p2p-node.
async fn submit_usage_report(report: SignedUsageReport) {

    let target_url = env::var("REPORT_USAGE_URL")
        .unwrap_or_else(|_| "http://localhost:8002/report_usage".to_string());

    // Construct the JSON-RPC request
    let rpc_request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "report_usage", // Or could be configurable
        params: report, // The SignedUsageReport is the parameter
        id: 1, // Simple ID, could use a counter or timestamp
    };

    info!("Submitting JSON-RPC usage report to: {}", target_url);

    // Use the global HTTP client instead of creating a new one
    match HTTP_CLIENT.post(target_url)
        .json(&rpc_request) // Send the JSON-RPC request object as JSON
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
    request_url: Option<&str>,
) {
    println!("--- Background Log: Response ---");
    let content_encoding = headers.get(CONTENT_ENCODING).and_then(|h| h.to_str().ok());

    match parser::decompress_body(content_encoding, &Bytes::from(log_buffer)) {
        Ok(decompressed_bytes) => {
            match parser::parse_json_and_extract_usage(&decompressed_bytes, request_url) {
                Ok((json_value, usage_option)) => {
                    // Log JSON
                    match serde_json::to_string_pretty(&json_value) {
                        Ok(pretty_json) => println!("Body (JSON): {} ", pretty_json),
                        Err(_) => println!("Body (Raw JSON): {:?}", json_value),
                    }
                    // Sign and submit usage if extracted
                    if let Some(usage_data) = usage_option {
                        // Log tool usage if present
                        if let Some(ref tool_use) = usage_data.tool_use {
                            info!(
                                "Tool use detected in response: {} (id: {})",
                                tool_use.name,
                                tool_use.id
                            );
                        }

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
    mut receiver: Receiver<Result<Bytes, ChannelError>>,
    headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>, // Accept Arc<SigningKey>
    request_url: Option<String>, // Add request URL parameter
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
            // Convert Option<String> to Option<&str> for the URL
            let url_ref = request_url.as_deref();
            process_and_log_regular_body(log_buffer, &headers, &signing_key, url_ref);
        }
    } else {
        warn!("Logging task stopped due to stream error. Logged {} bytes before error.", log_buffer.len());
    }
    info!("[{}] Background log task finished.", Utc::now().to_rfc3339());
}

/// Background task for SSE responses.
pub async fn log_sse_response_task(
    mut receiver: Receiver<parser::SSEChunk>,
    _headers: HeaderMap<HeaderValue>,
    signing_key: Arc<SigningKey>,
    request_url: Option<String>, // Add request URL parameter
) {
    let mut current_usage = UsageData::new();
    let mut final_usage_to_submit = UsageData::default();

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
                    final_usage_to_submit = current_usage.clone(); // Capture usage to sign
                    // --- Sign and Submit ---
                    let payload = UsageReportPayload {
                        usage: final_usage_to_submit.clone(), // Clone usage data
                        timestamp: Utc::now().timestamp(),
                    };

                    // Log tool usage information if present
                    if let Some(ref tool_use) = final_usage_to_submit.tool_use {
                        info!(
                            "Final SSE Usage includes tool use: {} (id: {})",
                            tool_use.name,
                            tool_use.id
                        );
                    }

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
            parser::SSEChunk::Text(text) => {
                trace!("Received SSE Text: {}", text);
            },
            parser::SSEChunk::Other(s) => {
                debug!("Received SSE Other: {}", s);
            },
        }
    }
    println!("--- SSE Event Stream Ended ---");
    info!("[{}] Background SSE log task finished.", Utc::now().to_rfc3339());
    info!("Final SSE Token Usage Submitted: {:?}", final_usage_to_submit);
}
