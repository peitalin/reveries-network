use bytes::Bytes;
use flate2::read::{GzDecoder, DeflateDecoder};
use std::io::{self, Read};
use serde_json::Value;
use std::error::Error;
use url::Url;
use std::fmt;
use tracing::{info, warn};
pub use crate::usage::UsageData;

mod anthropic;
mod deepseek;
mod sse_parser;

pub use anthropic::AnthropicParser;
pub use deepseek::DeepseekParser;
pub use sse_parser::{
    SSEParser,
    SSEChunk,
    parse_sse_data_line
};

/// Error type for parsing operations
#[derive(Debug)]
pub enum ParseError {
    Decompression(io::Error),
    JsonParsing(serde_json::Error),
    UrlParsing(url::ParseError),
    UnsupportedProvider,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Decompression(e) => write!(f, "Decompression failed: {}", e),
            ParseError::JsonParsing(e) => write!(f, "JSON parsing failed: {}", e),
            ParseError::UrlParsing(e) => write!(f, "URL parsing failed: {}", e),
            ParseError::UnsupportedProvider => write!(f, "Unsupported API provider"),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ParseError::Decompression(e) => Some(e),
            ParseError::JsonParsing(e) => Some(e),
            ParseError::UrlParsing(e) => Some(e),
            ParseError::UnsupportedProvider => None,
        }
    }
}

/// Trait for provider-specific parsers
pub trait UsageParser {
    /// Check if this parser can handle the given URL
    fn can_handle(&self, url: &str) -> bool;

    /// Extract usage data from a JSON response
    fn extract_usage(&self, json: &Value) -> Option<UsageData>;

    /// Parse SSE event data based on provider format
    /// By default, delegates to the generic SSE parser
    fn parse_sse_data(&self, data_str: &str) -> Vec<SSEChunk> {
        parse_sse_data_line(data_str)
    }
}


/// Parses decompressed bytes as JSON and extracts usage data based on API provider.
pub fn parse_json_and_extract_usage(
    bytes: &[u8],
    request_url: Option<&str>
) -> Result<(Value, Option<UsageData>), ParseError> {

    let json_value: Value = serde_json::from_slice(bytes)
        .map_err(ParseError::JsonParsing)?;

    let usage_data = if let Some(url) = request_url {
        match get_parser_for_url(url) {
            Ok(parser) => {
                let usage_opt = parser.extract_usage(&json_value);
                usage_opt
            },
            Err(_) => {
                warn!("No URL match. Falling back to generic usage extraction for URL: {}", url);
                extract_generic_usage(&json_value)
            }
        }
    } else {
        warn!("No request URL provided, falling back to generic usage extraction.");
        extract_generic_usage(&json_value)
    };

    Ok((json_value, usage_data))
}

pub fn get_parser_for_url(url: &str) -> Result<Box<dyn UsageParser>, ParseError> {
    info!("Attempting to get parser for URL: {}", url);
    // Try to parse URL
    let parsed_url = Url::parse(url).map_err(ParseError::UrlParsing)?;

    // Get hostname
    let host = parsed_url.host_str().ok_or(ParseError::UnsupportedProvider)?;
    let path = parsed_url.path();
    info!("Parsed host: '{}', path: '{}'", host, path);

    // Select parser based on hostname and path
    if host.contains("anthropic.com") || path.contains("/anthropic/") {
        info!("Selected AnthropicParser for URL: {}", url);
        Ok(Box::new(AnthropicParser))
    } else if host.contains("deepseek.com") || path.contains("/deepseek/") {
        info!("Selected DeepseekParser for URL: {}", url);
        Ok(Box::new(DeepseekParser))
    } else if host.contains("openai.com") || path.contains("/openai/") {
        warn!("OpenAI parser not yet implemented, using Anthropic as fallback for URL: {}", url);
        Ok(Box::new(AnthropicParser)) // Fallback to Anthropic parser for now
    } else {
        warn!("Unknown API provider for URL: {}. Returning UnsupportedProvider.", url);
        Err(ParseError::UnsupportedProvider)
    }
}

/// Generic extraction for when we don't have a specific parser
fn extract_generic_usage(json: &Value) -> Option<UsageData> {
    // Extract usage data from the 'usage' field in the JSON
    let usage_data = json.get("usage")
        .and_then(|u| u.as_object())
        .map(|usage_map| {
            let input = usage_map.get("input_tokens")
                .or_else(|| usage_map.get("prompt_tokens"))
                .and_then(|v| v.as_u64()).unwrap_or(0);

            let output = usage_map.get("output_tokens")
                .or_else(|| usage_map.get("completion_tokens"))
                .and_then(|v| v.as_u64()).unwrap_or(0);

            let cache_creation = usage_map.get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64());

            let cache_read = usage_map.get("cache_read_input_tokens")
                .or_else(|| usage_map.get("prompt_cache_hit_tokens"))
                .and_then(|v| v.as_u64());

            // Create the basic UsageData object with token counts
            UsageData {
                input_tokens: input,
                output_tokens: output,
                cache_creation_input_tokens: cache_creation,
                cache_read_input_tokens: cache_read,
                tool_use: None, // No tool use in generic extraction
            }
        })
        .filter(|u| u.input_tokens > 0 || u.output_tokens > 0)?;

    // Tool use lookup is more complex and provider-specific,
    // so we won't try to handle it in the generic extraction
    Some(usage_data)
}

/// Decompresses response body bytes based on Content-Encoding header.
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
