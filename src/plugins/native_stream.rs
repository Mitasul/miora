use crate::observability::usage::FactUsage;
use crate::protocols::provider_stream::{next_event_boundary, StreamOutcome};
use crate::transport::body::upstream_chunks;
use crate::transport::http::is_hop_by_hop;
use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use bytes::Bytes;
use futures_util::Stream;
use serde_json::Value;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

const OBSERVATION_LIMIT: usize = 1024 * 1024;

pub(crate) fn response_headers(headers: &HeaderMap) -> HeaderMap {
    let connection = headers
        .get_all("connection")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect::<Vec<_>>();
    headers
        .iter()
        .filter(|(name, _)| {
            !is_hop_by_hop(name.as_str())
                && name.as_str() != "content-length"
                && !connection
                    .iter()
                    .any(|hop| hop.eq_ignore_ascii_case(name.as_str()))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

pub(crate) fn relay(
    response: reqwest::Response,
    idle: Option<Duration>,
) -> (Response<Body>, oneshot::Receiver<StreamOutcome>) {
    let status = response.status();
    let headers = response_headers(response.headers());
    let media_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("");
    let sse = media_type.eq_ignore_ascii_case("text/event-stream");
    let observe =
        sse || media_type.eq_ignore_ascii_case("application/json") || media_type.ends_with("+json");
    let (sender, outcome) = oneshot::channel();
    let body = NativeBody {
        inner: Box::pin(upstream_chunks(response, idle)),
        sender: Some(sender),
        status,
        started: Instant::now(),
        first_output_ms: None,
        sse,
        observe,
        buffer: Vec::new(),
        usage: FactUsage::default(),
        semantic_failure: None,
    };
    let mut output = Response::new(Body::from_stream(body));
    *output.status_mut() = status;
    *output.headers_mut() = headers;
    (output, outcome)
}

struct NativeBody {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>>,
    sender: Option<oneshot::Sender<StreamOutcome>>,
    status: StatusCode,
    started: Instant,
    first_output_ms: Option<f64>,
    sse: bool,
    observe: bool,
    buffer: Vec<u8>,
    usage: FactUsage,
    semantic_failure: Option<&'static str>,
}

impl NativeBody {
    fn observe_json(&mut self, bytes: &[u8]) {
        if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
            if value
                .get("error")
                .is_some_and(|error| !error.is_null() && error != &Value::Bool(false))
                || matches!(
                    value.get("type").and_then(Value::as_str),
                    Some("error" | "response.failed" | "response.incomplete")
                )
                || matches!(
                    value.pointer("/response/status").and_then(Value::as_str),
                    Some("failed" | "incomplete" | "cancelled")
                )
            {
                self.semantic_failure = Some("upstream_protocol_error");
            }
            self.usage.merge(FactUsage::from_usage(
                value
                    .get("usage")
                    .or_else(|| value.pointer("/response/usage"))
                    .or_else(|| value.pointer("/message/usage"))
                    .or_else(|| value.get("usageMetadata")),
            ));
        }
    }

    fn observe_bytes(&mut self, bytes: &[u8]) {
        if !self.observe {
            return;
        }
        if self.buffer.len().saturating_add(bytes.len()) > OBSERVATION_LIMIT {
            self.buffer.clear();
            self.observe = false;
            return;
        }
        self.buffer.extend_from_slice(bytes);
        if self.sse {
            while let Some((end, separator)) = next_event_boundary(&self.buffer) {
                let event = self.buffer.drain(..end + separator).collect::<Vec<_>>();
                if String::from_utf8_lossy(&event).lines().any(|line| {
                    line.strip_prefix("event:")
                        .is_some_and(|event| event.trim() == "error")
                }) {
                    self.semantic_failure = Some("upstream_protocol_error");
                }
                let data = String::from_utf8_lossy(&event)
                    .lines()
                    .filter_map(|line| {
                        line.strip_prefix("data:")
                            .map(|line| line.strip_prefix(' ').unwrap_or(line))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                self.observe_json(data.as_bytes());
            }
        }
    }

    fn finish(&mut self, status: u16, failure_reason: Option<&'static str>) {
        if let Some(sender) = self.sender.take() {
            let input = self.usage.input.unwrap_or(0);
            let output = self.usage.output.unwrap_or(0);
            let _ = sender.send(StreamOutcome {
                usage: (input, output, input.saturating_add(output)),
                fact_usage: self.usage.clone(),
                success: self.status.is_success() && failure_reason.is_none(),
                observational_only: true,
                status_code: status,
                detail: String::new(),
                failure_reason,
                first_output_ms: self.first_output_ms,
                response_created_ms: None,
                first_text_ms: None,
            });
        }
    }
}

impl Stream for NativeBody {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(context) {
            Poll::Ready(Some(Ok(bytes))) => {
                if !bytes.is_empty() && self.first_output_ms.is_none() {
                    self.first_output_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
                }
                self.observe_bytes(&bytes);
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.finish(502, Some("transport_error"));
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                if self.observe && !self.sse {
                    let buffer = std::mem::take(&mut self.buffer);
                    self.observe_json(&buffer);
                }
                let status = self.status.as_u16();
                let failure = (!self.status.is_success())
                    .then_some("upstream_http_error")
                    .or(self.semantic_failure);
                self.finish(status, failure);
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for NativeBody {
    fn drop(&mut self) {
        self.finish(499, Some("client_cancelled"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn response(status: u16, content_type: &str, body: &[u8]) -> reqwest::Response {
        Response::builder()
            .status(status)
            .header("content-type", content_type)
            .body(body.to_vec())
            .unwrap()
            .into()
    }

    #[tokio::test]
    async fn native_proxy_errors_and_redirects_keep_original_status_body_and_headers() {
        for status in [202, 302, 401, 422, 429, 503] {
            let raw = b"\x1f\x8b\x00\xffcompressed-native-data";
            let mut upstream = response(status, "application/octet-stream", raw);
            upstream
                .headers_mut()
                .insert("content-encoding", "gzip".parse().unwrap());
            upstream
                .headers_mut()
                .insert("location", "/api/jobs/job-one".parse().unwrap());
            upstream
                .headers_mut()
                .insert("retry-after", "5".parse().unwrap());
            upstream
                .headers_mut()
                .append("set-cookie", "first=one".parse().unwrap());
            upstream
                .headers_mut()
                .append("set-cookie", "second=two".parse().unwrap());
            upstream
                .headers_mut()
                .insert("connection", "x-hop".parse().unwrap());
            upstream
                .headers_mut()
                .insert("x-hop", "private".parse().unwrap());
            let (output, outcome) = relay(upstream, None);
            assert_eq!(output.status().as_u16(), status);
            assert_eq!(output.headers()["content-encoding"], "gzip");
            assert_eq!(output.headers()["location"], "/api/jobs/job-one");
            assert_eq!(output.headers().get_all("set-cookie").iter().count(), 2);
            assert!(!output.headers().contains_key("x-hop"));
            assert_eq!(
                to_bytes(output.into_body(), 4096).await.unwrap().as_ref(),
                raw
            );
            let outcome = outcome.await.unwrap();
            assert_eq!(outcome.status_code, status);
            assert_eq!(outcome.success, status < 300);
            assert!(outcome.observational_only);
        }
    }

    #[tokio::test]
    async fn native_proxy_sse_stays_native_while_usage_and_soft_errors_are_observed() {
        let raw = b"event: vendor.thought\r\ndata: {\"vendor\":true}\r\n\r\ndata: {\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"input_tokens_details\":{\"cached_tokens\":8}}}\n\nevent: error\ndata: {\"type\":\"error\",\"error\":{\"message\":\"vendor-error\"}}\n\n";
        let (output, outcome) = relay(response(200, "text/event-stream", raw), None);
        assert_eq!(
            to_bytes(output.into_body(), 4096).await.unwrap().as_ref(),
            raw
        );
        let outcome = outcome.await.unwrap();
        assert_eq!(outcome.usage, (12, 3, 15));
        assert_eq!(outcome.fact_usage.cache_read, Some(8));
        assert!(!outcome.success);
        assert_eq!(outcome.status_code, 200);
        assert_eq!(outcome.failure_reason, Some("upstream_protocol_error"));
    }

    #[tokio::test]
    async fn native_proxy_json_usage_and_cancellation_do_not_rewrite_the_body() {
        let raw = b" {\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":4,\"cachedContentTokenCount\":6}} ";
        let (output, outcome) = relay(response(200, "application/json", raw), None);
        assert_eq!(
            to_bytes(output.into_body(), 4096).await.unwrap().as_ref(),
            raw
        );
        let outcome = outcome.await.unwrap();
        assert_eq!(outcome.usage, (10, 4, 14));
        assert_eq!(outcome.fact_usage.cache_read, Some(6));
        let (output, outcome) = relay(
            response(200, "text/event-stream", b"data: ongoing\n\n"),
            None,
        );
        drop(output);
        assert_eq!(outcome.await.unwrap().status_code, 499);
    }

    #[tokio::test]
    async fn native_proxy_observes_named_sse_errors_but_not_null_or_false_error_fields() {
        for raw in [&b"{\"error\":null}"[..], &b"{\"error\":false}"[..]] {
            let (output, outcome) = relay(response(200, "application/json", raw), None);
            assert_eq!(
                to_bytes(output.into_body(), 4096).await.unwrap().as_ref(),
                raw
            );
            assert!(outcome.await.unwrap().success);
        }
        let raw = b"event: error\ndata: {\"message\":\"native failure\"}\n\n";
        let (output, outcome) = relay(response(200, "text/event-stream", raw), None);
        assert_eq!(
            to_bytes(output.into_body(), 4096).await.unwrap().as_ref(),
            raw
        );
        assert!(!outcome.await.unwrap().success);
    }

    #[tokio::test]
    async fn native_proxy_observation_limit_never_truncates_native_content() {
        let raw = vec![b'x'; OBSERVATION_LIMIT + 1];
        let (output, outcome) = relay(response(200, "text/event-stream", &raw), None);
        assert_eq!(
            to_bytes(output.into_body(), raw.len())
                .await
                .unwrap()
                .as_ref(),
            raw
        );
        assert!(outcome.await.unwrap().success);
    }
}
