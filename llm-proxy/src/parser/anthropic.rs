use serde_json::Value;
use crate::usage::{UsageData, ToolUse};
use crate::parser::parse_sse_data_line;
use super::{UsageParser, SSEChunk};

/// Parser implementation for Anthropic API responses
pub struct AnthropicParser;

impl UsageParser for AnthropicParser {
    fn can_handle(&self, url: &str) -> bool {
        url.contains("anthropic.com") || url.contains("/anthropic/")
    }

    fn extract_usage(&self, json: &Value) -> Option<UsageData> {
        // First, extract basic token usage information
        let mut usage_data = json.get("usage")
            .and_then(|u| u.as_object())
            .map(|usage_map| {
                let input = usage_map.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                let output = usage_map.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

                let cache_creation = usage_map.get("cache_creation_input_tokens").and_then(|v| v.as_u64());
                let cache_read = usage_map.get("cache_read_input_tokens").and_then(|v| v.as_u64());

                UsageData {
                    input_tokens: input,
                    output_tokens: output,
                    cache_creation_input_tokens: cache_creation,
                    cache_read_input_tokens: cache_read,
                    tool_use: None,
                }
            })
            .filter(|u| u.input_tokens > 0 || u.output_tokens > 0)?;

        // Check for tool usage
        if json.get("stop_reason").and_then(|r| r.as_str()) == Some("tool_use") {
            if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
                // Find the first tool_use content block
                for item in content {
                    if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let id = item.get("id").and_then(|id| id.as_str()).unwrap_or("unknown").to_string();
                        let name = item.get("name").and_then(|name| name.as_str()).unwrap_or("unknown").to_string();
                        let input = item.get("input").cloned().unwrap_or(Value::Null);
                        // Create a ToolUse struct
                        usage_data.tool_use = Some(ToolUse {
                            id,
                            name,
                            input,
                            tool_type: "tool_use".to_string(),
                        });
                        // Only extract the first tool use
                        break;
                    }
                }
            }
        }

        Some(usage_data)
    }

    fn parse_sse_data(&self, data_str: &str) -> Vec<SSEChunk> {
        parse_sse_data_line(data_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_url_detection() {
        let parser = AnthropicParser;
        assert!(parser.can_handle("https://api.anthropic.com/v1/messages"));
        assert!(parser.can_handle("https://example.com/anthropic/v1/messages"));
        assert!(!parser.can_handle("https://api.openai.com/v1/chat/completions"));
    }

    #[test]
    fn test_anthropic_usage_extraction() {
        let parser = AnthropicParser;

        // Sample Anthropic response
        let json_str = r#"{
            "id": "msg_0123456789abcdef",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello, world!"}],
            "model": "claude-3-opus-20240229",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 250,
                "cache_creation_input_tokens": 50,
                "cache_read_input_tokens": null
            }
        }"#;

        let json: Value = serde_json::from_str(json_str).unwrap();
        let usage = parser.extract_usage(&json).unwrap();

        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 250);
        assert_eq!(usage.cache_creation_input_tokens, Some(50));
        assert_eq!(usage.cache_read_input_tokens, None);
    }
}