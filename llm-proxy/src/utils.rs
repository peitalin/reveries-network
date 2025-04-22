use bytes::Bytes;
use flate2::read::{GzDecoder, DeflateDecoder};
use std::io::{self, Read};
use serde_json::Value;
use std::fmt;
use std::error::Error;
pub use crate::usage::UsageData;

/// Error type for processing failures.
#[derive(Debug)]
pub enum ProcessError {
    Decompression(io::Error),
    JsonParsing(serde_json::Error),
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessError::Decompression(e) => write!(f, "Decompression failed: {}", e),
            ProcessError::JsonParsing(e) => write!(f, "JSON parsing failed: {}", e),
        }
    }
}

impl Error for ProcessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ProcessError::Decompression(e) => Some(e),
            ProcessError::JsonParsing(e) => Some(e),
        }
    }
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
) -> Result<Vec<u8>, ProcessError> {
    let mut decompressed_bytes = Vec::new();

    match content_encoding {
        Some("gzip") => {
            let mut decoder = GzDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes)
                .map_err(ProcessError::Decompression)?;
        },
        Some("deflate") => {
            let mut decoder = DeflateDecoder::new(&body_bytes[..]);
            decoder.read_to_end(&mut decompressed_bytes)
                .map_err(ProcessError::Decompression)?;
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
pub fn parse_json_and_extract_usage(
    bytes: &[u8]
) -> Result<(Value, Option<UsageData>), ProcessError> {

    let json_value: Value = serde_json::from_slice(bytes)
        .map_err(ProcessError::JsonParsing)?;

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

