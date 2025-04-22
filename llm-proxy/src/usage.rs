use bytes::Bytes;
use chrono::Utc;
use hudsucker::hyper::header::{CONTENT_ENCODING, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Receiver;
use tracing::{info, error, warn, debug, trace};

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

pub async fn save_usage_data(usage: UsageData) {
    tracing::info!(
        "Saving Usage - Input: {}, Output: {}",
        usage.input_tokens,
        usage.output_tokens
    );
    // In a real scenario, this would interact with a database, KV store, etc.
    // E.g.: db_client.record_usage("anthropic", usage.input_tokens, usage.output_tokens).await;
}

/// Processes the aggregated body of a non-SSE response.
pub fn process_and_log_regular_body(log_buffer: Vec<u8>, headers: &HeaderMap<HeaderValue>) {
    println!("--- Background Log: Response ---");
    let content_encoding = headers.get(CONTENT_ENCODING).and_then(|h| h.to_str().ok());

    match utils::decompress_body(content_encoding, &Bytes::from(log_buffer)) {
        Ok(decompressed_bytes) => {
            match utils::parse_json_and_extract_usage(&decompressed_bytes) {
                Ok((json_value, usage_option)) => {
                    // Pretty print the JSON
                    match serde_json::to_string_pretty(&json_value) {
                        Ok(pretty_json) => println!("Body (JSON): {} ", pretty_json),
                        Err(_) => println!("Body (Raw JSON): {:?}", json_value),
                    }
                    // Save usage if extracted
                    if let Some(usage_data) = usage_option {
                         tokio::spawn(save_usage_data(usage_data));
                    }
                },
                Err(e) => { // ProcessError covers JSON parsing errors
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
    mut receiver: Receiver<Result<Bytes, Box<dyn std::error::Error + Send + Sync>>>,
    headers: HeaderMap<HeaderValue>,
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
            process_and_log_regular_body(log_buffer, &headers);
        }
    } else {
        warn!("Logging task stopped due to stream error. Logged {} bytes before error.", log_buffer.len());
        // Optionally process partial buffer if desired
        // if !log_buffer.is_empty() { process_and_log_regular_body(log_buffer, &headers); }
    }
    info!("[{}] Background log task finished.", Utc::now().to_rfc3339());
}

/// Background task for SSE responses.
pub async fn log_sse_response_task(
    mut receiver: Receiver<SSEUpdate>,
    _headers: HeaderMap<HeaderValue>,
) {
    let mut current_usage = UsageData::new();
    let mut final_usage_to_log = UsageData::default(); // Store the usage logged by save_usage_data

    println!("--- Background Log: SSE Events Start ---");
    while let Some(update) = receiver.recv().await {
        debug!("Received SSE Update: {:?}", update);
        match update {
            SSEUpdate::UsageStart { input_tokens } => {
                current_usage.input_tokens = input_tokens;
            },
            SSEUpdate::UsageDelta { output_tokens } => {
                // Assuming this gives the final count as per Anthropic doc
                current_usage.output_tokens = output_tokens;
            },
            SSEUpdate::Stop => {
                if current_usage.input_tokens > 0 || current_usage.output_tokens > 0 {
                    final_usage_to_log = current_usage.clone(); // Capture usage *before* saving
                    tokio::spawn(save_usage_data(current_usage.clone()));
                    // Reset for potential future messages (though unlikely for one stream)
                    current_usage = UsageData::new();
                }
                // Optionally break the loop here
                // break;
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
    // Log the usage that was actually passed to save_usage_data
    info!("Final Token Usage Recorded: {:?}", final_usage_to_log);
}