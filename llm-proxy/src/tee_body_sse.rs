use bytes::Bytes;
use std::io;
use std::string::ToString;
use hudsucker::Body as HudsuckerBody;
use http_body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use std::error::Error as StdError;
use pin_project_lite::pin_project;
use http_body_util::BodyExt;
use tracing;
use serde_json::Value;

// A Body wrapper specifically for SSE streams.
// It parses SSE events and sends *complete events* through the channel.
pin_project! {
    #[derive(Debug)]
    pub struct TeeBodySSE<B: Body> {
        #[pin]
        inner: B,
        sender: mpsc::Sender<SSEUpdate>,
        parser: SSEParser,
    }
}

impl<B> TeeBodySSE<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    fn new(inner: B, sender: mpsc::Sender<SSEUpdate>) -> Self {
        Self {
            inner,
            sender,
            parser: SSEParser::new(),
        }
    }
}

// Implement the Body trait for TeeBodySSE
impl<B> Body for TeeBodySSE<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>> + Send + Sync + 'static,
{
    type Data = Bytes;
    type Error = Box<dyn StdError + Send + Sync + 'static>;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {

        let proj = self.project();
        match proj.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let updates = proj.parser.process_chunk(data);
                    for update in updates {
                        if proj.sender.try_send(update).is_err() {
                            tracing::warn!("SSE logging channel full or closed, dropping update.");
                        }
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

     fn is_end_stream(&self) -> bool {
         self.inner.is_end_stream()
     }

     fn size_hint(&self) -> SizeHint {
         self.inner.size_hint()
     }
}

/// Creates a TeeBodySSE wrapper and an MPSC receiver to capture parsed SSEUpdates.
pub fn tee_body_sse(body: HudsuckerBody) -> (HudsuckerBody, mpsc::Receiver<SSEUpdate>) {
    let (sender, receiver) = mpsc::channel(100);
    let teed_body = TeeBodySSE::new(body, sender);
    let mapped_body = teed_body.map_err(|e| {
        let io_err = io::Error::new(io::ErrorKind::Other, e.to_string());
        hudsucker::Error::Io(io_err)
    });
    let boxed_body = mapped_body.boxed();
    (HudsuckerBody::from(boxed_body), receiver)
}

/// Parses Server-Sent Events (SSE) from byte chunks.
#[derive(Debug)]
struct SSEParser {
    buffer: Vec<u8>,
}

impl SSEParser {
    fn new() -> Self {
        SSEParser { buffer: Vec::new() }
    }

    /// Processes an incoming chunk of bytes and returns a list of parsed SSE updates.
    fn process_chunk(&mut self, chunk: &Bytes) -> Vec<SSEUpdate> {
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
pub enum SSEUpdate {
    UsageStart { input_tokens: u64 },
    UsageDelta { output_tokens: u64 }, // Holds the final count from a delta event
    Text(String), // Holds text from content_block_delta
    Stop, // Represents message_stop event
    Other(String), // Store the type string for unhandled/other events
}

/// Parses a single SSE data line JSON string and extracts relevant updates.
fn parse_sse_data_line(data_str: &str) -> Vec<SSEUpdate> {
    let mut updates = Vec::new();

    // Parse into generic Value first
    match serde_json::from_str::<Value>(data_str) {
        Ok(json_event) => {
            // Extract type first
            if let Some(event_type) = json_event.get("type").and_then(|t| t.as_str()) {
                match event_type {
                    "message_start" => {
                        // Extract usage from message.usage.input_tokens
                        let opt_input = json_event.get("message")
                            .and_then(|m| m.get("usage"))
                            .and_then(|u| u.get("input_tokens"))
                            .and_then(|it| it.as_u64());

                        if let Some(input) = opt_input {
                            updates.push(SSEUpdate::UsageStart { input_tokens: input });
                        } else {
                            updates.push(SSEUpdate::Other(event_type.to_string()));
                        }
                    },
                    "content_block_delta" => {
                        // Extract text from delta.text
                        let opt_text = json_event.get("delta")
                            .and_then(|d| d.get("text"))
                            .and_then(|t| t.as_str());

                        if let Some(text) = opt_text {
                            updates.push(SSEUpdate::Text(text.to_string()));
                        } else {
                            updates.push(SSEUpdate::Other(event_type.to_string()));
                        }
                    },
                    "message_delta" => {
                        // Extract usage from top-level usage.output_tokens
                        let opt_output = json_event.get("usage")
                            .and_then(|u| u.get("output_tokens"))
                            .and_then(|ot| ot.as_u64());

                        if let Some(output) = opt_output {
                            updates.push(SSEUpdate::UsageDelta { output_tokens: output });
                        } else {
                            updates.push(SSEUpdate::Other(event_type.to_string()));
                        }
                    },
                    "message_stop" => {
                        updates.push(SSEUpdate::Stop);
                    },
                    // Known types we just pass as Other
                    "ping" | "content_block_start" | "content_block_stop" | "error" => {
                        updates.push(SSEUpdate::Other(event_type.to_string()));
                    },
                    unknown_type => {
                        tracing::debug!("Unknown SSE event data type: {}", unknown_type);
                        updates.push(SSEUpdate::Other(unknown_type.to_string()));
                    }
                }
            } else {
                tracing::warn!("SSE data line JSON missing 'type' field: {}", data_str);
                 updates.push(SSEUpdate::Other("MissingType".to_string()));
            }
        },
        Err(e) => {
            tracing::warn!("Failed to parse SSE data line as JSON: {}. Data: {}", e, data_str);
             updates.push(SSEUpdate::Other("ParseError".to_string()));
        }
    }
    updates
}

/// Parses raw SSE event strings (potentially multi-line, ending in \n\n) into SSEUpdates.
fn parse_sse_event_string(event_str: &str) -> Vec<SSEUpdate> {
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


