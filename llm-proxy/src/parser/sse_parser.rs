use bytes::Bytes;
use serde_json::Value;
use crate::parser::get_parser_for_url;


#[derive(Debug)]
pub struct SSEParser {
    buffer: Vec<u8>,
    request_url: Option<String>,
}

impl SSEParser {
    pub fn new(request_url: Option<&str>) -> Self {
        SSEParser {
            buffer: Vec::new(),
            request_url: request_url.map(|s| s.to_string()),
        }
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
                    all_updates.extend(self.parse_sse_event_string(&event_str));
                },
                Err(e) => {
                    tracing::warn!("Skipping non-UTF8 data segment: {}", e);
                }
            }
        }
        all_updates
    }

    /// Parse SSE event string with provider awareness
    fn parse_sse_event_string(&self, event_str: &str) -> Vec<SSEChunk> {
        let mut all_updates = Vec::new();
        for line in event_str.lines() {
            if line.starts_with("data:") {
                let data_part = line["data:".len()..].trim();
                if data_part.is_empty() { continue; }

                // Use provider-specific parser if URL is available
                if let Some(url) = &self.request_url {
                    if let Ok(parser) = get_parser_for_url(url) {
                        all_updates.extend(parser.parse_sse_data(data_part));
                        continue;
                    }
                }

                // Default to Anthropic format if no URL or parser available
                all_updates.extend(parse_sse_data_line(data_part));
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
    ToolUse {
        id: String,
        name: String,
        input: Value,
        tool_type: String
    }, // Tool use events (same structure as ToolUse struct)
    Other(String), // Store the type string for unhandled/other events
}

/// Parses a single SSE data line JSON string and extracts relevant updates.
pub fn parse_sse_data_line(data_str: &str) -> Vec<SSEChunk> {
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
        "content_block_start" => {
            // Check if this is a tool_use content block
            if let Some(_index) = json_event.get("index").and_then(|i| i.as_u64()) {
                if json_event.get("content_block").and_then(|c| c.get("type")).and_then(|t| t.as_str()) == Some("tool_use") {
                    let content_block = json_event.get("content_block").unwrap();
                    // Extract tool use details
                    let id = content_block.get("id").and_then(|id| id.as_str()).unwrap_or("unknown");
                    let name = content_block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                    let input = content_block.get("input").unwrap_or(&Value::Null);

                    tracing::info!("Found tool use event: id={}, name={}", id, name);

                    updates.push(SSEChunk::ToolUse {
                        id: id.to_string(),
                        name: name.to_string(),
                        input: input.clone(),
                        tool_type: "tool_use".to_string()
                    });
                }
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
        "ping" | "content_block_stop" | "error" => {
            updates.push(SSEChunk::Other(event_type.to_string()));
        },
        unknown_type => {
            tracing::debug!("Unknown SSE event data type: {}", unknown_type);
            updates.push(SSEChunk::Other(unknown_type.to_string()));
        }
    }

    updates
}