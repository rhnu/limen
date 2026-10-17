mod content;
mod error;
mod error_response;
mod events;
mod json;
mod lifecycle;
mod logic;
mod native;
mod observations;
mod observe;
mod persistence;
mod pipeline;
mod plan;
mod protocol;
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
pub(crate) use error::{ErrorCode, Failure};
pub(crate) use json::encode as encode_json;
use lifecycle::Lifecycle;
use limen_config::{BusinessConfig, ResourceLimits};
use limen_script::{Cancellation, RuntimeStatus, ScriptRuntime};
pub(crate) use logic::{authorize, models_json, safe_target};
pub(crate) use persistence::RecordSink;
pub(crate) use plan::{Error as PlanError, ExecutionPlans};
use response::Delivery;
use tokio::{
    sync::{Mutex, Semaphore, oneshot},
    task::JoinSet,
};
use tracing::Instrument;
pub(crate) use transport::{FixtureResponse, Trace, Transport};
pub(crate) use upstream::media_kind;

/// Only generation configuration enters the request engine.
pub(crate) struct GenerationConfig {
    pub limits: ResourceLimits,
    pub client_headers: Vec<String>,
    pub observation_names: Vec<String>,
    pub storage_operation_ms: u64,
    pub business: Arc<BusinessConfig>,
    pub plans: Arc<ExecutionPlans>,
}

pub(crate) struct GenerationServices {
    pub runtime: ScriptRuntime,
    pub transport: Transport,
    pub records: RecordSink,
    pub shutdown: Cancellation,
    pub run_id: String,
}

/// Owns bounded generation lifetimes and their upstream and persistence operations.
pub(crate) struct Gateway {
    limits: ResourceLimits,
    client_headers: Vec<String>,
    observation_names: Vec<String>,
    storage_operation_ms: u64,
    business: Arc<BusinessConfig>,
    plans: Arc<ExecutionPlans>,
    runtime: ScriptRuntime,
    transport: Transport,
    shutdown: Cancellation,
    permits: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    calls: AtomicU64,
    run_id: String,
    records: RecordSink,
}

impl Gateway {
    pub(crate) fn new(config: GenerationConfig, services: GenerationServices) -> Arc<Self> {
        let permits = Arc::new(Semaphore::new(config.limits.concurrency));
        Arc::new(Self {
            limits: config.limits,
            client_headers: config.client_headers,
            observation_names: config.observation_names,
            storage_operation_ms: config.storage_operation_ms,
            business: config.business,
            plans: config.plans,
            runtime: services.runtime,
            transport: services.transport,
            shutdown: services.shutdown,
            permits,
            tasks: Mutex::new(JoinSet::new()),
            calls: AtomicU64::new(1),
            run_id: services.run_id,
            records: services.records,
        })
    }

    /// The worker survives a dropped HTTP future only long enough to record its terminal
    /// outcome and run bounded observations. Its permit covers that cleanup as well.
    pub(crate) async fn handle_generation(
        self: Arc<Self>,
        request: Request<Body>,
        request_id: String,
        connection_cancel: Cancellation,
    ) -> Response {
        let Ok(permit) = Arc::clone(&self.permits).try_acquire_owned() else {
            return error_response(&Failure::new(ErrorCode::Busy, "admission"), &request_id);
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
        tasks.spawn(
            async move {
                let _permit = permit;
                let mut lifecycle = gateway.lifecycle(id, cancellation);
                let mut delivery = Delivery::new(
                    reply,
                    lifecycle.cancellation.clone(),
                    lifecycle.request_id.clone(),
                );
                gateway
                    .run_request(
                        request,
                        &mut lifecycle,
                        &mut delivery,
                        worker_connection_cancel,
                    )
                    .await;
            }
            .instrument(span),
        );
        drop(tasks);
        let response = tokio::select! {
            biased;
            () = connection_cancel.cancelled() => {
                if let Some(cancel) = &cancel_on_drop.0 { cancel.cancel(); }
                error_response(&Failure::new(ErrorCode::Cancelled, "connection"), &request_id)
            },
            result = receive => result.unwrap_or_else(|_| error_response(&Failure::new(ErrorCode::Internal, "request_task"), &request_id)),
        };
        // Streaming bodies carry their own cancellation guard beyond handle_generation().
        cancel_on_drop.0.take();
        response
    }

    async fn run_request(
        &self,
        request: Request<Body>,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
        worker_connection_cancel: Cancellation,
    ) {
        delivery.timings = lifecycle.statistics.timings.clone();
        let result = self
            .execute_request(request, lifecycle, delivery, &worker_connection_cancel)
            .await;
        let result = self
            .request_active(lifecycle, &worker_connection_cancel)
            .and(result);
        let cleanup_deadline = Instant::now() + Duration::from_millis(self.limits.cleanup_ms);
        lifecycle
            .statistics
            .freeze(lifecycle.started, Instant::now());
        let record = persistence::terminal(lifecycle, result.as_ref().err());
        let persist = async {
            if let Some(record) = record {
                let deadline = Instant::now() + Duration::from_millis(self.storage_operation_ms);
                if let Err(error) = self.records.finish(record, deadline).await {
                    tracing::error!(
                        phase = "storage_finish",
                        code = error.code(),
                        "request persistence failed"
                    );
                }
            }
        };
        let cleanup = async {
            let terminal = match result {
                Ok(response) => {
                    if let Some(response) = response {
                        delivery.send_json(response);
                    }
                    Ok(())
                },
                Err(mut failure) => {
                    if delivery.pending() && lifecycle.recorded {
                        self.prepare_error(&mut failure, lifecycle, cleanup_deadline)
                            .await;
                    }
                    // Terminal writes have a finite budget even after the request
                    // deadline; a slow or disconnected consumer cannot retain a permit.
                    let _finished = tokio::time::timeout_at(
                        tokio::time::Instant::from_std(cleanup_deadline),
                        delivery.fail(&failure, lifecycle),
                    )
                    .await;
                    delivery.close();
                    Err(failure)
                },
            };
            if lifecycle.recorded {
                self.observe(lifecycle, terminal.as_ref().err(), cleanup_deadline)
                    .await;
            }
            lifecycle.record(&terminal, &self.observation_names);
            self.transport
                .complete(lifecycle, terminal.as_ref().err(), &self.observation_names)
                .await;
        };
        tokio::join!(persist, cleanup);
    }

    async fn execute_request(
        &self,
        request: Request<Body>,
        lifecycle: &mut Lifecycle,
        delivery: &mut Delivery,
        connection_cancel: &Cancellation,
    ) -> Result<Option<Response>, Failure> {
        let cancellation = lifecycle.cancellation.clone();
        let deadline = tokio::time::Instant::from_std(lifecycle.deadline);
        let body = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = self.shutdown.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = connection_cancel.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = tokio::time::sleep_until(deadline) => Err(Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase)),
            result = self.prepare_request(request, lifecycle) => result,
        }?;
        self.request_active(lifecycle, connection_cancel)?;
        // Once submitted, retain the bounded write confirmation even when the client
        // disconnects. Cancellation is a request result, not a storage failure.
        self.record_begin(lifecycle).await?;
        self.request_active(lifecycle, connection_cancel)?;
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = self.shutdown.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = connection_cancel.cancelled() => Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase)),
            () = tokio::time::sleep_until(deadline) => Err(Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase)),
            result = self.complete(body, lifecycle, delivery) => result,
        }
    }

    fn request_active(
        &self,
        lifecycle: &Lifecycle,
        connection_cancel: &Cancellation,
    ) -> Result<(), Failure> {
        if lifecycle.cancellation.is_cancelled()
            || self.shutdown.is_cancelled()
            || connection_cancel.is_cancelled()
        {
            Err(Failure::new(ErrorCode::Cancelled, lifecycle.phase))
        } else if Instant::now() >= lifecycle.deadline {
            Err(Failure::new(ErrorCode::DeadlineExceeded, lifecycle.phase))
        } else {
            Ok(())
        }
    }

    /// Shutdown stops admission before draining all tracked requests and observations.
    pub(crate) async fn drain(&self) {
        self.permits.close();
        let mut tasks = self.tasks.lock().await;
        while let Some(result) = tasks.join_next().await {
            report_task(result);
        }
    }

    fn next_call(&self) -> u64 { self.calls.fetch_add(1, Ordering::Relaxed) }

    fn lifecycle(&self, id: String, cancellation: Cancellation) -> Lifecycle {
        let mut lifecycle = Lifecycle::new(id, cancellation, self.limits.request_ms);
        lifecycle.observations =
            observations::Observations::new(self.limits.max_observations_per_request);
        lifecycle
    }

    pub(crate) fn accepting(&self) -> bool {
        !self.permits.is_closed()
            && !self.shutdown.is_cancelled()
            && matches!(self.runtime.status(), RuntimeStatus::Ready)
    }

    fn hook_deadline(&self, lifecycle: &Lifecycle) -> Instant {
        lifecycle
            .deadline
            .min(Instant::now() + Duration::from_millis(self.limits.hook_ms))
    }
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

pub(crate) fn error_response(failure: &Failure, request_id: &str) -> Response {
    error_response_with_headers(failure, request_id, http::HeaderMap::new())
}

fn error_response_with_headers(
    failure: &Failure,
    request_id: &str,
    mut headers: http::HeaderMap,
) -> Response {
    if let Some(payload) = &failure.response {
        if let Some(value) = &payload.content_type {
            headers.insert(header::CONTENT_TYPE, value.clone());
        }
        for value in &payload.content_encoding {
            headers.append(header::CONTENT_ENCODING, value.clone());
        }
        return raw_response(
            failure.http_status(),
            headers,
            payload.body.clone(),
            request_id,
        );
    }
    // The Rust-owned envelope has fixed messages and a gateway-assigned ID. Its
    // small encoding remains independent of user-configured model output limits.
    json_response(
        failure.http_status(),
        headers,
        failure.error_json(request_id).to_string().into_bytes(),
        request_id,
    )
}

pub(crate) fn json_response(
    status: u16,
    mut headers: http::HeaderMap,
    body: Vec<u8>,
    request_id: &str,
) -> Response {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    raw_response(status, headers, body, request_id)
}

fn raw_response(
    status: u16,
    mut headers: http::HeaderMap,
    body: Vec<u8>,
    request_id: &str,
) -> Response {
    if let Ok(id) = HeaderValue::from_str(request_id) {
        headers.insert("x-limen-request-id", id);
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() =
        http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    *response.headers_mut() = headers;
    response
}
