mod error;
mod events;
mod json;
mod lifecycle;
mod logic;
mod observations;
mod observe;
mod pipeline;
mod response;
mod stats;
mod transport;
mod upstream;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    http::{HeaderValue, Request, header},
    response::Response,
};
use error::{ErrorCode, Failure};
use lifecycle::Lifecycle;
use limen_config::{BusinessConfig, ServerConfig};
use limen_script::{Cancellation, RuntimeStatus, ScriptRuntime};
use response::Delivery;
use tokio::{
    sync::{Mutex, Semaphore, oneshot},
    task::JoinSet,
};
use tracing::Instrument;
pub(crate) use transport::{FixtureResponse, Trace};
pub(crate) use upstream::media_kind;

use crate::bootstrap::Bootstrap;

/// Owns the connection pool and the bounded, tracked request lifetimes.
pub struct Gateway {
    server: ServerConfig,
    business: BusinessConfig,
    runtime: ScriptRuntime,
    transport: transport::Transport,
    shutdown: Cancellation,
    permits: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    requests: AtomicU64,
    calls: AtomicU64,
}

impl Gateway {
    pub fn new(loaded: &Bootstrap, shutdown: Cancellation) -> Result<Arc<Self>, reqwest::Error> {
        // Generation is sent once. Disable even protocol-level retries and redirects.
        let pooled_origins = loaded
            .business
            .providers
            .values()
            .map(|provider| logic::safe_target(&provider.endpoint))
            .collect();
        let transport = transport::Transport::http(pooled_origins)?;
        Ok(Self::with_transport(loaded, shutdown, transport))
    }

    pub(crate) fn offline(
        loaded: &Bootstrap,
        response: FixtureResponse,
    ) -> (Arc<Self>, Arc<Mutex<Trace>>) {
        let (transport, trace) = transport::Transport::fixture(response);
        (
            Self::with_transport(loaded, Cancellation::new(), transport),
            trace,
        )
    }

    fn with_transport(
        loaded: &Bootstrap,
        shutdown: Cancellation,
        transport: transport::Transport,
    ) -> Arc<Self> {
        Arc::new(Self {
            server: loaded.server.clone(),
            business: loaded.business.clone(),
            runtime: loaded.runtime.clone(),
            transport,
            shutdown,
            permits: Arc::new(Semaphore::new(loaded.server.limits.concurrency)),
            tasks: Mutex::new(JoinSet::new()),
            requests: AtomicU64::new(1),
            calls: AtomicU64::new(1),
        })
    }

    /// The worker survives a dropped HTTP future only long enough to record its terminal
    /// outcome and run bounded observations. Its permit covers that cleanup as well.
    pub async fn handle(
        self: Arc<Self>,
        request: Request<Body>,
        connection_cancel: Cancellation,
    ) -> Response {
        let request_id = format!("limen-{}", self.requests.fetch_add(1, Ordering::Relaxed));
        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            let failure = Failure::new(ErrorCode::Busy, "admission");
            let mut lifecycle = self.lifecycle(request_id.clone(), Cancellation::new());
            lifecycle
                .statistics
                .freeze(lifecycle.started, Instant::now());
            // HTTP/1 drivers bound these rejected lifetimes too. Their observations
            // await inline and cannot create an unbounded second task queue.
            self.observe(
                &mut lifecycle,
                Some(&failure),
                Instant::now() + Duration::from_millis(self.server.limits.cleanup_ms),
            )
            .await;
            lifecycle.record(&Err(failure), &self.server.observability.events);
            let failure = Failure::new(ErrorCode::Busy, "admission");
            return error_response(&failure, &request_id);
        };
        let cancellation = Cancellation::new();
        let mut cancel_on_drop = CancelOnDrop(Some(cancellation.clone()));
        let (reply, receive) = oneshot::channel();
        let gateway = Arc::clone(&self);
        let id = request_id.clone();
        let span = tracing::info_span!("request", request_id = %id);
        let mut tasks = self.tasks.lock().await;
        while let Some(result) = tasks.try_join_next() {
            report_task(result);
        }
        let worker_connection_cancel = connection_cancel.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let mut lifecycle = gateway.lifecycle(id, cancellation);
            let cancellation = lifecycle.cancellation.clone();
            let deadline = lifecycle.deadline;
            let mut delivery = Delivery::new(reply, cancellation.clone(), lifecycle.request_id.clone());
            delivery.timings = lifecycle.statistics.timings.clone();
            let result = tokio::select! {
                biased;
                () = cancellation.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
                () = gateway.shutdown.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
                () = worker_connection_cancel.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
                () = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => Err(Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase)),
                result = gateway.process(request, &mut lifecycle, &mut delivery) => result,
            };
            let result = if lifecycle.cancellation.is_cancelled() || gateway.shutdown.is_cancelled() {
                Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase))
            } else if Instant::now() >= lifecycle.deadline {
                Err(Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase))
            } else { result };
            let cleanup_deadline = Instant::now() + Duration::from_millis(gateway.server.limits.cleanup_ms);
            lifecycle.statistics.freeze(lifecycle.started, Instant::now());
            let terminal = match result {
                Ok(response) => {
                    if let Some(response) = response { delivery.send_json(response); }
                    Ok(())
                },
                Err(failure) => {
                    // Terminal writes have a finite budget even after the request
                    // deadline; a slow or disconnected consumer cannot retain a permit.
                    let _finished = tokio::time::timeout_at(
                        tokio::time::Instant::from_std(cleanup_deadline),
                        delivery.fail(&failure, &lifecycle),
                    ).await;
                    delivery.close();
                    Err(failure)
                },
            };
            gateway.observe(&mut lifecycle, terminal.as_ref().err(), cleanup_deadline).await;
            lifecycle.record(&terminal, &gateway.server.observability.events);
            gateway.transport.complete(&lifecycle, terminal.as_ref().err(), &gateway.server.observability.events).await;
        }.instrument(span));
        drop(tasks);
        let response = tokio::select! {
            biased;
            () = connection_cancel.cancelled() => {
                if let Some(cancel) = &cancel_on_drop.0 { cancel.cancel(); }
                error_response(&Failure::new(ErrorCode::Cancelled, "connection"), &request_id)
            },
            result = receive => result.unwrap_or_else(|_| error_response(&Failure::new(ErrorCode::Internal, "request_task"), &request_id)),
        };
        // Streaming bodies carry their own cancellation guard beyond handle().
        cancel_on_drop.0.take();
        response
    }

    /// Shutdown stops admission before draining all tracked requests and observations.
    pub async fn drain(&self) {
        self.permits.close();
        let mut tasks = self.tasks.lock().await;
        while let Some(result) = tasks.join_next().await {
            report_task(result);
        }
    }

    fn next_call(&self) -> u64 { self.calls.fetch_add(1, Ordering::Relaxed) }

    fn lifecycle(&self, id: String, cancellation: Cancellation) -> Lifecycle {
        let mut lifecycle = Lifecycle::new(id, cancellation, self.server.limits.request_ms);
        lifecycle.observations =
            observations::Observations::new(self.server.limits.max_observations_per_request);
        lifecycle
    }

    fn ready(&self) -> bool {
        !self.shutdown.is_cancelled() && matches!(self.runtime.status(), RuntimeStatus::Ready)
    }

    fn hook_deadline(&self, lifecycle: &Lifecycle) -> Instant {
        lifecycle
            .deadline
            .min(Instant::now() + Duration::from_millis(self.server.limits.hook_ms))
    }
}

fn upstream_client(max_idle: usize) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .pool_max_idle_per_host(max_idle)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()
}

struct CancelOnDrop(Option<Cancellation>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.cancel();
        }
    }
}

fn report_task(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::error!(
            panicked = error.is_panic(),
            cancelled = error.is_cancelled(),
            "request worker failed; payload omitted"
        );
    }
}

fn error_response(failure: &Failure, request_id: &str) -> Response {
    error_response_with_headers(failure, request_id, http::HeaderMap::new())
}

fn error_response_with_headers(
    failure: &Failure,
    request_id: &str,
    headers: http::HeaderMap,
) -> Response {
    // The Rust-owned envelope has fixed messages and a gateway-assigned ID. Its
    // small encoding remains independent of user-configured model output limits.
    json_response(
        failure.http_status(),
        headers,
        failure.error_json(request_id).to_string().into_bytes(),
        request_id,
    )
}

fn json_response(
    status: u16,
    mut headers: http::HeaderMap,
    body: Vec<u8>,
    request_id: &str,
) -> Response {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Ok(id) = HeaderValue::from_str(request_id) {
        headers.insert("x-limen-request-id", id);
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() =
        http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    *response.headers_mut() = headers;
    response
}
