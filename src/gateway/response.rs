use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::{Body, Bytes, HttpBody},
    http::{HeaderMap, HeaderValue, header},
    response::Response,
};
use hyper::body::Frame;
use limen_script::{Cancellation, HookCandidate, HookInput};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{
    Gateway,
    error::{ErrorCode, Failure},
    lifecycle::Lifecycle,
    pipeline::{commit_effects, hook_invalid},
    plan::RoutePlan,
    stats::{Timings, WaitKind},
};

// Rust-owned error envelopes have a fixed bound independent of the model data cap.
const CONTROL_EVENT_BYTES: usize = 1024;

/// A single producer owns the bounded body until the HTTP consumer drains or drops it.
/// Keeping the acknowledgement separate from queue capacity covers the final frame too.
pub(super) struct Delivery {
    reply: Option<oneshot::Sender<Response>>,
    sender: Option<mpsc::Sender<QueuedFrame>>,
    drained: Option<oneshot::Receiver<()>>,
    cancellation: Cancellation,
    status: u16,
    headers: HeaderMap,
    request_id: String,
    done_sent: bool,
    pub(super) timings: Timings,
    pub(super) started: Instant,
}

impl Delivery {
    pub fn new(
        reply: oneshot::Sender<Response>,
        cancellation: Cancellation,
        request_id: String,
    ) -> Self {
        Self {
            reply: Some(reply),
            sender: None,
            drained: None,
            cancellation,
            status: 200,
            headers: HeaderMap::new(),
            request_id,
            done_sent: false,
            timings: Timings::default(),
            started: Instant::now(),
        }
    }

    pub fn configure(&mut self, status: u16, headers: HeaderMap) {
        self.status = status;
        self.headers = headers;
    }

    pub fn pending(&self) -> bool { self.reply.is_some() }

    pub fn send_json(&mut self, response: Response) {
        if let Some(reply) = self.reply.take() {
            let _delivered = reply.send(response);
        }
    }

    fn start(&mut self) -> Result<(), Failure> {
        if self.sender.is_some() {
            return Ok(());
        }
        let Some(reply) = self.reply.take() else {
            return Err(Failure::new(ErrorCode::Cancelled, "response"));
        };
        let (sender, receiver) = mpsc::channel(1);
        let (finished, drained) = oneshot::channel();
        let body = StreamBody {
            receiver,
            finished: Some(finished),
            cancellation: self.cancellation.clone(),
            completed: false,
            timings: self.timings.clone(),
            started: self.started,
        };
        let mut response = Response::new(Body::new(body));
        *response.status_mut() = http::StatusCode::from_u16(self.status)
            .map_err(|source| Failure::new(ErrorCode::Internal, "response").with_source(source))?;
        *response.headers_mut() = self.headers.clone();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        if let Ok(id) = HeaderValue::from_str(&self.request_id) {
            response.headers_mut().insert("x-limen-request-id", id);
        }
        reply
            .send(response)
            .map_err(|_undelivered_response| Failure::new(ErrorCode::Cancelled, "response"))?;
        self.sender = Some(sender);
        self.drained = Some(drained);
        Ok(())
    }

    pub async fn emit(&mut self, bytes: Vec<u8>) -> Result<(), Failure> {
        self.emit_content(bytes, limen_chat::GeneratedContent::default())
            .await
    }

    pub async fn emit_content(
        &mut self,
        bytes: Vec<u8>,
        content: limen_chat::GeneratedContent,
    ) -> Result<(), Failure> {
        self.start()?;
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| Failure::new(ErrorCode::Internal, "response"))?;
        let _measurement = self.timings.measure(WaitKind::Queue);
        sender
            .send(QueuedFrame {
                bytes: Bytes::from(bytes),
                content,
            })
            .await
            .map_err(|source| Failure::new(ErrorCode::Cancelled, "response").with_source(source))
    }

    pub async fn finish(&mut self, terminator: Option<&limen_chat::Frame>) -> Result<(), Failure> {
        self.emit(
            terminator.map_or_else(|| limen_chat::DONE.to_vec(), |frame| frame.raw().to_vec()),
        )
        .await?;
        self.done_sent = true;
        self.sender.take();
        if let Some(drained) = self.drained.take() {
            let _measurement = self.timings.measure(WaitKind::Drain);
            drained.await.map_err(|source| {
                Failure::new(ErrorCode::Cancelled, "response").with_source(source)
            })?;
        }
        Ok(())
    }

    pub fn completed(&self, deadline: Instant) -> bool {
        self.done_sent
            && self
                .timings
                .drained_at()
                .is_some_and(|drained_at| drained_at <= deadline)
    }

    pub fn close(&mut self) {
        self.sender.take();
        self.drained.take();
    }

    /// Error delivery and observations share the caller's finite cleanup deadline.
    pub async fn fail(&mut self, failure: &Failure, lifecycle: &Lifecycle) {
        if self.reply.is_some() {
            self.send_json(super::error_response_with_headers(
                failure,
                &lifecycle.request_id,
                lifecycle.response_headers.clone(),
            ));
        } else if self.sender.is_some() && !self.done_sent && failure.code != ErrorCode::Cancelled {
            let native = failure.response.as_ref().and_then(|response| {
                limen_config::parse_json_slice(&response.body, limen_config::MAX_JSON_DEPTH).ok()
            });
            let envelope = native.unwrap_or_else(|| failure.error_json(&lifecycle.request_id));
            // Native structured errors were bounded at the protocol boundary.
            // If their encoded framing cannot fit, still terminate without DONE.
            let limit = failure
                .response
                .as_ref()
                .map_or(CONTROL_EVENT_BYTES, |response| {
                    response
                        .body
                        .len()
                        .saturating_add(64)
                        .max(CONTROL_EVENT_BYTES)
                });
            if let Ok(bytes) = limen_chat::encode_error(&envelope, limit) {
                let _sent = self.emit(bytes).await;
            }
        }
        self.sender.take();
        if let Some(drained) = self.drained.take() {
            let _drained = drained.await;
        }
    }
}

struct QueuedFrame {
    bytes: Bytes,
    content: limen_chat::GeneratedContent,
}

struct StreamBody {
    receiver: mpsc::Receiver<QueuedFrame>,
    timings: Timings,
    started: Instant,
    finished: Option<oneshot::Sender<()>>,
    cancellation: Cancellation,
    completed: bool,
}

impl HttpBody for StreamBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match self.receiver.poll_recv(cx) {
            Poll::Ready(Some(frame)) => {
                self.timings.downstream(
                    self.started,
                    Instant::now(),
                    frame.content.generation,
                    frame.content.text,
                );
                Poll::Ready(Some(Ok(Frame::data(frame.bytes))))
            },
            Poll::Ready(None) => {
                self.timings.drained(Instant::now());
                self.completed = true;
                if let Some(finished) = self.finished.take() {
                    let _sent = finished.send(());
                }
                Poll::Ready(None)
            },
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Drop for StreamBody {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
    }
}

impl Gateway {
    pub(super) async fn response_body(
        &self,
        route: &RoutePlan<'_>,
        mut body: Value,
        lifecycle: &mut Lifecycle,
    ) -> Result<Value, Failure> {
        apply_alias(&mut body, lifecycle);
        let (provider_steps, model_steps) = route.response();
        for step in provider_steps.iter().chain(model_steps) {
            lifecycle.scope = step.scope;
            let hook = step.executor;
            let context = lifecycle.context(HookInput::OnResponse { body });
            let output = self
                .hook(hook, Some(&step.name), context, lifecycle)
                .await?;
            let HookCandidate::OnResponse { body: next_body } = output.candidate else {
                return Err(hook_invalid("onResponse", &step.name));
            };
            commit_effects(lifecycle, output.effects);
            body = next_body;
        }
        Ok(body)
    }
}

pub(super) fn apply_alias(body: &mut Value, lifecycle: &Lifecycle) {
    if body.get("model").is_some() {
        body["model"] = json!(lifecycle.alias);
    }
}

#[cfg(test)]
mod tests;
