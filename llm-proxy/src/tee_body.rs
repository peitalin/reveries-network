use bytes::Bytes;
use hudsucker::Body as HudsuckerBody; // Alias to avoid naming conflicts
use http_body::{Body, Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc; // For the channel
use std::error::Error as StdError;
use http_body_util::BodyExt; // Import BodyExt trait for .boxed()
use pin_project_lite::pin_project;

// A Body wrapper that clones data frames and sends them through an MPSC channel.
pin_project! {
    pub struct TeeBody<B: Body> {
        #[pin]
        inner: B,
        sender: mpsc::Sender<Result<Bytes, Box<dyn StdError + Send + Sync>>>,
    }
}

impl<B> TeeBody<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    pub fn new(inner: B, sender: mpsc::Sender<Result<Bytes, Box<dyn StdError + Send + Sync>>>) -> Self {
        Self { inner, sender }
    }
}

impl<B> Body for TeeBody<B>
where
    B: Body<Data = Bytes> + Unpin,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    type Data = Bytes;
    type Error = B::Error; // Propagate the inner body's error type

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        match this.inner.poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    let _ = this.sender.try_send(Ok(data.clone()));
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => {
                // Convert the error *representation* for the channel
                // We need a way to represent the error without consuming `e`
                // Assuming B::Error implements Display or Debug might work, or Clone.
                // Let's just send a generic error message for now if we can't clone/copy `e`.
                // Or, ideally, B::Error implements Clone. Let's assume it might not.
                // A simple approach: just return the error, don't try to send it via channel.
                // Let the receiver task handle potential abrupt stream end.
                // Simpler: Don't send error via channel, just return it.
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                Poll::Ready(None)
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

/// Creates a TeeBody wrapper and an MPSC receiver to capture body chunks.
pub fn tee_body(body: HudsuckerBody) -> (HudsuckerBody, mpsc::Receiver<Result<Bytes, Box<dyn StdError + Send + Sync>>>) {
    // Create a channel with a buffer size (e.g., 100). Adjust as needed.
    // If the logger task falls behind, `try_send` in TeeBody will start dropping chunks.
    let (sender, receiver) = mpsc::channel(100);
    let teed_body = TeeBody::new(body, sender);
    // HudsuckerBody implements From<BoxBody>, so we box our TeeBody
    let boxed_body = teed_body.boxed();
    (HudsuckerBody::from(boxed_body), receiver) // Explicitly convert using From
}