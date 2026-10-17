//! Read-only management uses independent admission and owned query permits.
mod config;
mod query;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, header},
    response::Response,
};
pub(crate) use config::AdminConfigSnapshot;
use limen_script::RuntimeStatus;
use limen_storage::{HealthState, StorageReader};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use crate::{
    routes::AdminRoute,
    telemetry::{
        LoggingHealth,
        reader::{LogReader, ReadError},
    },
};

const MAX_RESPONSE: usize = 1024 * 1024;

pub(crate) struct ManagementSettings {
    pub concurrency: usize,
    pub timeout: Duration,
}

pub(crate) struct AdminServices {
    pub run_id: String,
    pub started_at: String,
    pub runtime_status: watch::Receiver<RuntimeStatus>,
    pub storage: StorageReader,
    pub logging: LoggingHealth,
    pub logs: LogReader,
}

pub(crate) struct Admin {
    settings: ManagementSettings,
    configuration: AdminConfigSnapshot,
    runtime_status: watch::Receiver<RuntimeStatus>,
    run_id: String,
    started_at: String,
    storage: StorageReader,
    logging: LoggingHealth,
    logs: LogReader,
    permits: Arc<Semaphore>,
}

#[derive(Debug, PartialEq, Eq)]
enum Error {
    InvalidQuery,
    CursorExpired,
    TooBroad,
    Unavailable,
    Busy,
    Timeout,
    NotFound,
    Method,
}

impl From<limen_storage::Error> for Error {
    fn from(error: limen_storage::Error) -> Self {
        match error {
            limen_storage::Error::Invalid => Self::InvalidQuery,
            limen_storage::Error::TooBroad => Self::TooBroad,
            limen_storage::Error::Timeout => Self::Timeout,
            limen_storage::Error::Busy => Self::Busy,
            _ => Self::Unavailable,
        }
    }
}

impl From<ReadError> for Error {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::InvalidQuery => Self::InvalidQuery,
            ReadError::CursorExpired => Self::CursorExpired,
            ReadError::Timeout => Self::Timeout,
            ReadError::Io(_) => Self::Unavailable,
        }
    }
}

impl Admin {
    pub async fn drain(&self) {
        // Blocking readers keep permits after their HTTP future expires. Shutdown
        // must wait for their actual completion within the caller's HTTP budget.
        if let Ok(_all) = self
            .permits
            .acquire_many(
                u32::try_from(self.settings.concurrency)
                    .expect("validated management concurrency fits u32"),
            )
            .await
        {
            self.permits.close();
        }
    }

    pub(crate) fn new(
        settings: ManagementSettings,
        configuration: AdminConfigSnapshot,
        services: AdminServices,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(settings.concurrency));
        Self {
            settings,
            configuration,
            runtime_status: services.runtime_status,
            run_id: services.run_id,
            started_at: services.started_at,
            storage: services.storage,
            logging: services.logging,
            logs: services.logs,
            permits,
        }
    }

    pub async fn handle(
        &self,
        route: AdminRoute,
        request: Request<Body>,
        id: &str,
        accepting: bool,
    ) -> Response {
        let deadline = Instant::now() + self.settings.timeout;
        let result = match Arc::clone(&self.permits).try_acquire_owned() {
            Ok(permit) => tokio::time::timeout_at(
                tokio::time::Instant::from_std(deadline),
                self.execute(route, request, accepting, permit, deadline),
            )
            .await
            .unwrap_or(Err(Error::Timeout)),
            Err(_) => Err(Error::Busy),
        };
        response(result, id)
    }

    async fn execute(
        &self,
        route: AdminRoute,
        request: Request<Body>,
        accepting: bool,
        permit: OwnedSemaphorePermit,
        deadline: Instant,
    ) -> Result<Value, Error> {
        if route == AdminRoute::Unknown {
            return Err(Error::NotFound);
        }
        if request.method() != Method::GET {
            return Err(Error::Method);
        }
        let parameters = query::parameters(request.uri().query().unwrap_or(""))?;
        to_bytes(request.into_body(), 0)
            .await
            .map_err(|_error| Error::InvalidQuery)?;
        let data = match route {
            AdminRoute::Status => {
                query::allowed(&parameters, &[])?;
                let storage = self.storage.health();
                let vm = match &*self.runtime_status.borrow() {
                    RuntimeStatus::Ready => "ready",
                    RuntimeStatus::Starting => "starting",
                    RuntimeStatus::Stopped => "stopped",
                    RuntimeStatus::Retired(_) => "retired",
                };
                json!({"data":{"version":env!("CARGO_PKG_VERSION"),"runId":self.run_id,"startedAt":self.started_at,"acceptingGenerations":accepting && storage.state==HealthState::Ready,"vm":vm,"storage":storage,"logging":self.logging.status()}})
            },
            AdminRoute::Config => self.configuration.page(&parameters, &self.run_id)?,
            AdminRoute::Requests => self.request_page(&parameters, permit, deadline).await?,
            AdminRoute::Usage => {
                let request = query::usage(&parameters)?;
                json!({"data":self.storage.usage(request, deadline, permit).await?})
            },
            AdminRoute::Logs | AdminRoute::LogFiles => {
                let reader = self.logs.clone();
                let files = route == AdminRoute::LogFiles;
                if files {
                    query::allowed(&parameters, &[])?;
                }
                let task = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    if files {
                        reader.files(deadline)
                    } else {
                        reader.logs(&parameters, deadline)
                    }
                });
                task.await.map_err(|_error| Error::Unavailable)??
            },
            AdminRoute::Request(id) => {
                query::allowed(&parameters, &[])?;
                if id.len() > 128 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return Err(Error::InvalidQuery);
                }
                let record = self
                    .storage
                    .get(id, deadline, permit)
                    .await?
                    .ok_or(Error::NotFound)?;
                json!({"data":record})
            },
            AdminRoute::Unknown => return Err(Error::NotFound),
        };
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        Ok(data)
    }

    async fn request_page(
        &self,
        parameters: &query::Parameters,
        permit: OwnedSemaphorePermit,
        deadline: Instant,
    ) -> Result<Value, Error> {
        let (request, cursor) = query::requests(parameters, &self.run_id)?;
        let mut page = self.storage.list(request, deadline, permit).await?;
        let filter = cursor.filter;
        // A cursor follows the last delivered record, including a page shortened by byte budget.
        let mut next = page.next;
        loop {
            let next_cursor = next
                .map(|position| {
                    query::encode(&query::RequestCursor {
                        version: 1,
                        run_id: self.run_id.clone(),
                        filter: filter.clone(),
                        high_watermark: page.high_watermark,
                        position,
                    })
                })
                .transpose()?;
            let result = json!({"data":page.records,"nextCursor":next_cursor});
            if serde_json::to_vec(&result)
                .map_err(|_error| Error::Unavailable)?
                .len()
                <= MAX_RESPONSE
            {
                return Ok(result);
            }
            if page.records.len() <= 1 {
                return Err(Error::TooBroad);
            }
            page.records.pop();
            next = page.records.last().map(|record| limen_storage::Position {
                started_at_ms: record.started_at_ms,
                record_id: record.record_id,
            });
        }
    }
}

fn response(result: Result<Value, Error>, id: &str) -> Response {
    let (status, value, method) = match result {
        Ok(value) => (200, value, false),
        Err(error) => {
            let (status, code, message) = match error {
                Error::InvalidQuery => (400, "invalid_query", "Invalid management query"),
                Error::CursorExpired => (410, "cursor_expired", "The query cursor has expired"),
                Error::TooBroad => (
                    422,
                    "query_too_broad",
                    "The query exceeds its bounded scope",
                ),
                Error::Unavailable => (
                    503,
                    "storage_unavailable",
                    "The requested data is unavailable",
                ),
                Error::Busy => (503, "busy", "The management service is busy"),
                Error::Timeout => (504, "query_timeout", "The query deadline was exceeded"),
                Error::NotFound => (404, "not_found", "The endpoint or record was not found"),
                Error::Method => (
                    405,
                    "method_not_allowed",
                    "The request method is not allowed",
                ),
            };
            (
                status,
                json!({"error":{"code":code,"message":message,"request_id":id}}),
                error == Error::Method,
            )
        },
    };
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    if bytes.len() > MAX_RESPONSE {
        return response(Err(Error::TooBroad), id);
    }
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() =
        http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(id) = HeaderValue::from_str(id) {
        headers.insert("x-limen-request-id", id);
    }
    if method {
        headers.insert(header::ALLOW, HeaderValue::from_static("GET"));
    }
    response
}
