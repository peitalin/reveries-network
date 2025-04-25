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
use crate::parser::{SSEParser, SSEChunk};

// A Body wrapper specifically for SSE streams.
// It parses SSE events and sends *complete events* through the channel.
pin_project! {
    pub struct TeeBodySSE<B: Body> {
        #[pin]
        inner: B,
        sender: mpsc::Sender<SSEChunk>,
        parser: SSEParser,
    }
}

impl<B> TeeBodySSE<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    fn new(inner: B, sender: mpsc::Sender<SSEChunk>, request_url: Option<&str>) -> Self {
        Self {
            inner,
            sender,
            parser: SSEParser::new(request_url),
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

/// Creates a TeeBodySSE wrapper and an MPSC receiver to capture parsed SSEChunks.
pub fn tee_body_sse(body: HudsuckerBody, request_url: Option<&str>) -> (HudsuckerBody, mpsc::Receiver<SSEChunk>) {
    let (sender, receiver) = mpsc::channel(100);
    let teed_body = TeeBodySSE::new(body, sender, request_url);
    let mapped_body = teed_body.map_err(|e| {
        let io_err = io::Error::new(io::ErrorKind::Other, e.to_string());
        hudsucker::Error::Io(io_err)
    });
    let boxed_body = mapped_body.boxed();
    (HudsuckerBody::from(boxed_body), receiver)
}
