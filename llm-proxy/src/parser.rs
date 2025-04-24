use bytes::Bytes;
use flate2::read::{GzDecoder, DeflateDecoder};
use std::io::{self, Read};
use serde_json::Value;
use std::fmt;
use std::error::Error;
pub use crate::usage::UsageData;


/// Parses Server-Sent Events (SSE) from byte chunks.
#[derive(Debug)]
pub struct SSEParser {
    buffer: Vec<u8>,
}

impl SSEParser {
    pub fn new() -> Self {
        SSEParser { buffer: Vec::new() }
    }

    /// Processes an incoming chunk of bytes and returns a list of parsed SSE updates.
    pub fn process_chunk(&mut self, chunk: &Bytes) -> Vec<SSEChunk> {
        self.buffer.extend_from_slice(chunk);
        let mut all_updates = Vec::new();
        // check if two consecutive \n\n (standard SSE Event separator) are present in buffer
        while let Some(index) = self.buffer.windows(2).position(|window| window == b"\n\n") {
            // then remove bytes up to SSE event separator index+2
            let event_data = self.buffer.drain(..index + 2).collect::<Vec<u8>>();
            match String::from_utf8(event_data) {
                Ok(event_str) => {
                    all_updates.extend(parse_sse_event_string(&event_str));
                },
                Err(e) => {
                    tracing::warn!("Skipping non-UTF8 data segment: {}", e);
                }
            }
        }
        all_updates
    }
}

/// Represents the relevant information extracted from a parsed SSE event data line.
#[derive(Debug, Clone)]
pub enum SSEChunk {
    InputTokens { input_tokens: u64 },
    OutputTokens { output_tokens: u64 }, // Holds the final count from a delta event
    Stop, // Represents message_stop event
    Text(String), // Holds the text from a content_block_delta event
    Other(String), // Store the type string for unhandled/other events
}

/// Parses raw SSE event strings (potentially multi-line, ending in \n\n) into SSEChunks.
fn parse_sse_event_string(event_str: &str) -> Vec<SSEChunk> {
    let mut all_updates = Vec::new();
    for line in event_str.lines() {
        if line.starts_with("data:") {
            let data_part = line["data:".len()..].trim();
            if data_part.is_empty() { continue; }
            all_updates.extend(parse_sse_data_line(data_part));
        }
    }
    all_updates
}

/// Parses a single SSE data line JSON string and extracts relevant updates.
fn parse_sse_data_line(data_str: &str) -> Vec<SSEChunk> {
    let mut updates = Vec::new();

    // Parse into generic Value first
    let json_event = match serde_json::from_str::<Value>(data_str) {
        Ok(json_event) => json_event,
        Err(e) => {
            tracing::warn!("Failed to parse SSE data line as JSON: {}. Data: {}", e, data_str);
            return vec![SSEChunk::Other("ParseError".to_string())];
        }
    };

    // Extract type
    let event_type = match json_event.get("type").and_then(|t| t.as_str()) {
        Some(event_type) => event_type,
        None => {
            tracing::warn!("SSE data line JSON missing 'type' field: {}", data_str);
            return vec![SSEChunk::Other("MissingType".to_string())];
        }
    };

    match event_type {
        "message_start" => {
            // Extract usage from message.usage.input_tokens
            let opt_input = json_event.get("message")
                .and_then(|m| m.get("usage"))
                .and_then(|u| u.get("input_tokens"))
                .and_then(|it| it.as_u64());

            if let Some(input) = opt_input {
                updates.push(SSEChunk::InputTokens { input_tokens: input });
            } else {
                updates.push(SSEChunk::Other(event_type.to_string()));
            }
        },
        "content_block_delta" => {
            // Extract text from delta.text
            let opt_text = json_event.get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str());

            if let Some(text) = opt_text {
                updates.push(SSEChunk::Text(text.to_string()));
            } else {
                updates.push(SSEChunk::Other(event_type.to_string()));
            }
        },
        "message_delta" => {
            // Extract usage from top-level usage.output_tokens
            let opt_output = json_event.get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|ot| ot.as_u64());

            if let Some(output) = opt_output {
                updates.push(SSEChunk::OutputTokens { output_tokens: output });
            } else {
                updates.push(SSEChunk::Other(event_type.to_string()));
            }
        },
        "message_stop" => {
            updates.push(SSEChunk::Stop);
        },
        // Known types we just pass as Other
        "ping" | "content_block_start" | "content_block_stop" | "error" => {
            updates.push(SSEChunk::Other(event_type.to_string()));
        },
        unknown_type => {
            tracing::debug!("Unknown SSE event data type: {}", unknown_type);
            updates.push(SSEChunk::Other(unknown_type.to_string()));
        }
    }

    updates
}


/// Decompresses response body bytes based on Content-Encoding header.
///
/// Args:
///     content_encoding: Optional string slice representing the Content-Encoding header value.
///     body_bytes: Bytes object containing the raw response body.
///
/// Returns:
///     Result containing a Vec<u8> of the decompressed bytes, or an io::Error if decompression fails.
pub fn decompress_body(
    content_encoding: Option<&str>,
    body_bytes: &Bytes
) -> Result<Vec<u8>, ParseError> {
    let mut decompressed_bytes = Vec::new();

    match content_encoding {
        Some("gzip") => {
            let mut decoder = GzDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes).map_err(ParseError::Decompression)?;
        },
        Some("deflate") => {
            let mut decoder = DeflateDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes).map_err(ParseError::Decompression)?;
        },
        _ => {
            // No compression or unknown, copy original bytes
            decompressed_bytes.extend_from_slice(body_bytes);
        }
    }
    Ok(decompressed_bytes)
}

/// Parses decompressed bytes as JSON and extracts usage data.
///
/// Args:
///     bytes: A slice of bytes containing the decompressed response body.
///
/// Returns:
///     Result containing a tuple of (Value, Option<UsageData>), where Value is the parsed JSON,
///     and Option<UsageData> is the extracted usage data if available.
pub fn parse_json_and_extract_usage(bytes: &[u8]) -> Result<(Value, Option<UsageData>), ParseError> {

    let json_value: Value = serde_json::from_slice(bytes)
        .map_err(ParseError::JsonParsing)?;

    let usage_data = json_value
        .get("usage")
        .and_then(|u| u.as_object())
        .map(|usage_map| {
            let input = usage_map.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let output = usage_map.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            UsageData {
                input_tokens: input,
                output_tokens: output,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            }
        })
        // Filter out empty usage data
        .filter(|u| u.input_tokens > 0 || u.output_tokens > 0);

    Ok((json_value, usage_data))
}


#[derive(Debug)]
pub enum ParseError {
    Decompression(io::Error),
    JsonParsing(serde_json::Error),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Decompression(e) => write!(f, "Decompression failed: {}", e),
            ParseError::JsonParsing(e) => write!(f, "JSON parsing failed: {}", e),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ParseError::Decompression(e) => Some(e),
            ParseError::JsonParsing(e) => Some(e),
        }
    }
}