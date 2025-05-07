use serde_json::Value;
use crate::usage::UsageData;
use crate::parser::parse_sse_data_line;
use super::UsageParser;
use super::SSEChunk;

pub struct DeepseekParser;

impl UsageParser for DeepseekParser {
    fn can_handle(&self, url: &str) -> bool {
        url.contains("deepseek.com") || url.contains("/deepseek/")
    }

    fn extract_usage(&self, json: &Value) -> Option<UsageData> {
        json.get("usage")
            .and_then(|u| u.as_object())
            .map(|usage_map| {
                // DeepSeek uses different field names for the same concepts
                let input = usage_map.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output = usage_map.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

                // DeepSeek-specific cache fields
                let cache_read = usage_map.get("prompt_cache_hit_tokens").and_then(|v| v.as_u64());
                let cache_miss = usage_map.get("prompt_cache_miss_tokens").and_then(|v| v.as_u64());

                // Note: We map prompt_cache_miss_tokens to cache_creation_input_tokens
                // This is a reasonable approximation, though not a perfect match
                UsageData {
                    reverie_id: None,
                    spender: None,
                    spender_type: None,
                    input_tokens: input,
                    output_tokens: output,
                    cache_creation_input_tokens: cache_miss,
                    cache_read_input_tokens: cache_read,
                    tool_use: None,
                }
            })
            .filter(|u| u.input_tokens > 0 || u.output_tokens > 0)
    }

    // Implement parse_sse_data for DeepSeek's SSE format
    // Note: Currently, DeepSeek doesn't seem to use SSE, but we'll implement a placeholder
    // that can be customized later if needed
    fn parse_sse_data(&self, data_str: &str) -> Vec<SSEChunk> {
        let mut updates = Vec::new();

        // Try to parse as JSON first
        if let Ok(json) = serde_json::from_str::<Value>(data_str) {
            // DeepSeek might use a different event structure
            // This is a placeholder implementation that would need to be customized
            // once we know DeepSeek's actual SSE format

            // Check for specific DeepSeek event types and handle them
            if let Some(event_type) = json.get("type").and_then(|t| t.as_str()) {
                match event_type {
                    // If we don't have DeepSeek-specific handling yet,
                    // just return as an "Other" type
                    _ => updates.push(SSEChunk::Other(event_type.to_string())),
                }

                // Extract tokens if available
                if let Some(usage) = json.get("usage") {
                    if let Some(input) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                        updates.push(SSEChunk::InputTokens { input_tokens: input });
                    }

                    if let Some(output) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                        updates.push(SSEChunk::OutputTokens { output_tokens: output });
                    }
                }

                // Extract content if available
                if let Some(content) = json.get("delta").and_then(|d| d.get("content")).and_then(|c| c.as_str()) {
                    updates.push(SSEChunk::Text(content.to_string()));
                }
            }
        }

        // If no DeepSeek-specific parsing was done, fall back to the default
        if updates.is_empty() {
            updates = parse_sse_data_line(data_str);
        }

        updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deepseek_url_detection() {
        let parser = DeepseekParser;
        assert!(parser.can_handle("https://api.deepseek.com/v1/chat/completions"));
        assert!(parser.can_handle("https://example.com/deepseek/v1/chat"));
        assert!(!parser.can_handle("https://api.anthropic.com/v1/messages"));
    }

    #[test]
    fn test_deepseek_usage_extraction() {
        let parser = DeepseekParser;

        // Sample DeepSeek response based on the example
        let json_str = r#"{
            "choices": [
                {
                    "finish_reason": "stop",
                    "index": 0,
                    "logprobs": null,
                    "message": {
                        "content": "Sample response content",
                        "role": "assistant"
                    }
                }
            ],
            "created": 1745485546,
            "id": "625ffbc3-a8fc-490c-87bb-8181b8235120",
            "model": "deepseek-chat",
            "object": "chat.completion",
            "system_fingerprint": "fp_8802369eaa_prod0225",
            "usage": {
                "completion_tokens": 192,
                "prompt_cache_hit_tokens": 0,
                "prompt_cache_miss_tokens": 434,
                "prompt_tokens": 434,
                "prompt_tokens_details": {
                    "cached_tokens": 0
                },
                "total_tokens": 626
            }
        }"#;

        let json: Value = serde_json::from_str(json_str).unwrap();
        let usage = parser.extract_usage(&json).unwrap();

        assert_eq!(usage.input_tokens, 434);
        assert_eq!(usage.output_tokens, 192);
        assert_eq!(usage.cache_creation_input_tokens, Some(434)); // prompt_cache_miss_tokens
        assert_eq!(usage.cache_read_input_tokens, Some(0));       // prompt_cache_hit_tokens
    }
}