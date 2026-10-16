use super::registry::Snapshot;
use axum::body::Body;
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Immutable request-wide registry generation. Clones keep all candidate
/// versions and the interceptor chain alive across fallback and hedged sends.
#[derive(Clone)]
pub(crate) struct PluginLease {
    pub snapshot: Arc<Snapshot>,
    pub service: bool,
}
impl PluginLease {
    pub fn retain(self, body: Body) -> Body {
        Body::new(LeasedBody {
            inner: Box::pin(body),
            lease: Some(self),
        })
    }
}
struct LeasedBody {
    inner: Pin<Box<Body>>,
    lease: Option<PluginLease>,
}
impl HttpBody for LeasedBody {
    type Data = Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, axum::Error>>> {
        let result = self.inner.as_mut().poll_frame(cx);
        if matches!(result, Poll::Ready(None) | Poll::Ready(Some(Err(_)))) {
            self.lease.take();
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
