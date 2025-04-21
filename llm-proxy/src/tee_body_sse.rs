use bytes::Bytes;
use std::io;
use hudsucker::Body as HudsuckerBody;
use http_body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use std::error::Error as StdError;
use pin_project_lite::pin_project;
use http_body_util::BodyExt;
use tracing;

/// Parses Server-Sent Events (SSE) from byte chunks.
#[derive(Debug)]
struct SseEventParser {
    buffer: Vec<u8>, // Buffer for incomplete lines/events
}

impl SseEventParser {
    fn new() -> Self {
        SseEventParser { buffer: Vec::new() }
    }

    /// Processes an incoming chunk of bytes and returns a list of complete SSE events found.
    fn process_chunk(&mut self, chunk: &Bytes) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some(index) = self.buffer.windows(2).position(|window| window == b"\n\n") {
            let event_data = self.buffer.drain(..index + 2).collect::<Vec<u8>>();
            // Attempt to convert the raw event data to a string
            match String::from_utf8(event_data) {
                Ok(event_str) => {
                    // Basic check: Ensure it contains "data:" or other SSE fields
                    if event_str.contains("data:") || event_str.contains("event:") || event_str.contains("id:") || event_str.trim().is_empty() {
                        events.push(event_str);
                    } else if !event_str.trim().is_empty() {
                        // Log potentially unexpected non-empty data that doesn't fit SSE format
                        tracing::trace!("Skipping potential non-SSE data segment: {:?}", event_str.trim());
                    }
                },
                Err(e) => {
                    // Log non-UTF8 data if needed
                    tracing::warn!("Skipping non-UTF8 data segment: {}", e);
                }
            }
        }
        events
    }
}


// A Body wrapper specifically for SSE streams.
// It parses SSE events and sends *complete events* through the channel.
pin_project! {
    #[derive(Debug)] // Add Debug for easier inspection if needed
    pub struct TeeBodySSE<B: Body> {
        #[pin]
        inner: B,
        sender: mpsc::Sender<String>, // Sends complete event strings
        parser: SseEventParser,
    }
}

impl<B> TeeBodySSE<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    fn new(inner: B, sender: mpsc::Sender<String>) -> Self {
        Self {
            inner,
            sender,
            parser: SseEventParser::new(),
        }
    }
}

// Implement the Body trait for TeeBodySSE
impl<B> Body for TeeBodySSE<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>> + Send + Sync + 'static, // Added bounds for boxing
{
    type Data = Bytes;
    // Box the error type to handle different potential inner errors generically
    type Error = Box<dyn StdError + Send + Sync + 'static>;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let proj = self.project();
        match proj.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    // Process the chunk and get complete events
                    let events = proj.parser.process_chunk(data);
                    for event in events {
                        // Try sending the complete event string.
                        // If the channel is full or closed, we drop the event silently.
                        // A production system might want more robust handling.
                        if proj.sender.try_send(event).is_err() {
                            tracing::warn!("SSE logging channel full or closed, dropping event.");
                            // Optionally break or handle backpressure differently here.
                        }
                    }
                }
                // Map the inner frame to our boxed error type
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => {
                 // Box the inner error and return it
                 Poll::Ready(Some(Err(e.into())))
            }
            Poll::Ready(None) => {
                // Handle any remaining data in the parser buffer when stream ends?
                // For simplicity, we assume valid SSE streams end cleanly.
                Poll::Ready(None) // Signal end of stream
            }
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


/// Creates a TeeBodySSE wrapper and an MPSC receiver to capture parsed SSE events.
pub fn tee_body_sse(body: HudsuckerBody) -> (HudsuckerBody, mpsc::Receiver<String>) {
    // Channel buffer size for parsed events
    let (sender, receiver) = mpsc::channel(100);
    let teed_body = TeeBodySSE::new(body, sender);
    // Map the error to a string and wrap in hudsucker::Error::Io
    let mapped_body = teed_body.map_err(|e| {
        let io_err = io::Error::new(io::ErrorKind::Other, e.to_string());
        hudsucker::Error::Io(io_err)
    });
    // Box the mapped TeeBodySSE to return a HudsuckerBody
    let boxed_body = mapped_body.boxed();
    (HudsuckerBody::from(boxed_body), receiver)
}
