use std::{convert::Infallible, fmt, sync::Arc, time::Duration};

use axum::body::Body;
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use limen_script::{Cancellation, RuntimeStatus, ScriptError};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinSet,
};

use crate::{bootstrap::Bootstrap, gateway::Gateway};

#[derive(thiserror::Error)]
pub enum Error {
    #[error("cannot bind HTTP listener")]
    Bind(#[source] std::io::Error),
    #[error("HTTP accept failed")]
    Accept(#[source] std::io::Error),
    #[error("cannot initialize upstream client")]
    Client(#[source] reqwest::Error),
    #[error("cannot wait for shutdown signal")]
    Signal(#[source] std::io::Error),
    #[error("JavaScript runtime retired")]
    Retired(#[source] ScriptError),
    #[error("JavaScript runtime stopped unexpectedly")]
    RuntimeStopped,
    #[error("persistent service resources are missing")]
    Resources,
    #[error("HTTP connection task failed")]
    Task(#[source] tokio::task::JoinError),
    #[error("HTTP shutdown budget exceeded")]
    ShutdownTimeout(#[source] tokio::time::error::Elapsed),
    #[error("HTTP failed and request cleanup also failed")]
    Cleanup {
        #[source]
        primary: Box<Self>,
        cleanup: Box<Self>,
    },
}

/// HTTP sources may contain URLs or client bytes; render only approved categories.
pub struct Diagnostic<'a>(&'a Error);

impl Error {
    pub fn diagnostic(&self) -> Diagnostic<'_> { Diagnostic(self) }

    pub fn phase(&self) -> &'static str {
        match self {
            Self::Bind(_) | Self::Client(_) | Self::Resources => "http_startup",
            Self::Accept(_) | Self::Task(_) => "http_transport",
            Self::Signal(_) | Self::ShutdownTimeout(_) => "http_shutdown",
            Self::Retired(_) | Self::RuntimeStopped => "script_runtime",
            Self::Cleanup { primary, .. } => primary.phase(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.diagnostic(), formatter)
    }
}

impl fmt::Display for Diagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Error::Bind(source) | Error::Accept(source) | Error::Signal(source) => {
                write!(formatter, "{}: IO {:?}", self.0, source.kind())
            },
            Error::Retired(source) => write!(formatter, "{}: {}", self.0, source.diagnostic()),
            Error::Task(source) => write!(
                formatter,
                "{}: {}",
                self.0,
                if source.is_panic() {
                    "panic"
                } else {
                    "cancelled"
                }
            ),
            Error::Cleanup { primary, cleanup } => write!(
                formatter,
                "{}; cleanup also failed: {}",
                primary.diagnostic(),
                cleanup.diagnostic()
            ),
            Error::Client(_)
            | Error::RuntimeStopped
            | Error::ShutdownTimeout(_)
            | Error::Resources => fmt::Display::fmt(self.0, formatter),
        }
    }
}

/// Bound transport tasks separately from request admission inside the gateway.
pub async fn serve(loaded: &Bootstrap) -> Result<(), Error> {
    let shutdown = Cancellation::new();
    let storage = loaded.storage.as_ref().ok_or(Error::Resources)?.clone();
    let logging = loaded.logging.as_ref().ok_or(Error::Resources)?;
    let gateway =
        Gateway::new(loaded, shutdown.clone(), storage, logging).map_err(Error::Client)?;
    let mut status = loaded.runtime.status_changes();
    runtime_ready(&status.borrow())?;
    let listener = TcpListener::bind(loaded.server.listen)
        .await
        .map_err(Error::Bind)?;
    let connection_limit = loaded.server.limits.concurrency + loaded.server.limits.queue_capacity;
    let header_timeout = Duration::from_millis(loaded.server.limits.request_ms);
    let transport_budget = header_timeout + Duration::from_millis(loaded.server.limits.cleanup_ms);
    let signal = shutdown_signal();
    tokio::pin!(signal);
    let mut connections = JoinSet::new();
    tracing::info!(listen = %loaded.server.listen, connection_limit, "HTTP listener started");
    let primary = loop {
        tokio::select! {
            biased;
            result = &mut signal => break result.map_err(Error::Signal),
            changed = status.changed() => {
                if changed.is_err() {
                    break Err(Error::RuntimeStopped);
                }
                if let Err(error) = runtime_ready(&status.borrow()) {
                    break Err(error);
                }
            },
            joined = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(source)) = joined {
                    break Err(Error::Task(source));
                }
            },
            accepted = listener.accept(), if connections.len() < connection_limit => {
                match accepted {
                    Ok((stream, _peer)) => {
                        connections.spawn(connection(stream, Arc::clone(&gateway), shutdown.clone(), header_timeout, transport_budget));
                    },
                    Err(source) => break Err(Error::Accept(source)),
                }
            },
        }
    };
    drop(listener);
    shutdown.cancel();
    tracing::info!(
        connections = connections.len(),
        "HTTP listener stopped; cancelling requests"
    );
    let cleanup = tokio::time::timeout(
        Duration::from_millis(loaded.server.limits.shutdown_ms),
        async {
            let mut failure = None;
            while let Some(joined) = connections.join_next().await {
                if let Err(source) = joined {
                    failure.get_or_insert(Error::Task(source));
                }
            }
            gateway.drain().await;
            failure.map_or(Ok(()), Err)
        },
    )
    .await
    .map_err(Error::ShutdownTimeout)
    .and_then(std::convert::identity);
    // Aborting a driver drops its cancellation guard, while gateway-owned cleanup
    // tasks remain tracked for the bounded shutdown result above.
    connections.abort_all();
    with_cleanup(primary, cleanup)
}

fn runtime_ready(status: &RuntimeStatus) -> Result<(), Error> {
    match status {
        RuntimeStatus::Ready => Ok(()),
        RuntimeStatus::Retired(source) => Err(Error::Retired(source.clone())),
        RuntimeStatus::Starting | RuntimeStatus::Stopped => Err(Error::RuntimeStopped),
    }
}

fn with_cleanup(primary: Result<(), Error>, cleanup: Result<(), Error>) -> Result<(), Error> {
    match (primary, cleanup) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(primary), Err(cleanup)) => Err(Error::Cleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        }),
    }
}

struct CancelOnDrop(Cancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) { self.0.cancel(); }
}

async fn connection(
    stream: TcpStream,
    gateway: Arc<Gateway>,
    shutdown: Cancellation,
    header_timeout: Duration,
    transport_budget: Duration,
) {
    let cancel = Cancellation::new();
    let _cancel_on_drop = CancelOnDrop(cancel.clone());
    let (deadline_tx, mut deadlines) = watch::channel(None);
    let service = service_fn(move |request: hyper::Request<Incoming>| {
        deadline_tx.send_replace(Some(tokio::time::Instant::now() + transport_budget));
        let gateway = Arc::clone(&gateway);
        let cancel = cancel.clone();
        async move { Ok::<_, Infallible>(gateway.handle(request.map(Body::new), cancel).await) }
    });
    let mut builder = http1::Builder::new();
    builder
        .half_close(false)
        .max_headers(100)
        .max_buf_size(32 * 1024)
        .max_header_size(32 * 1024)
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout);
    let driver = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(driver);
    let mut draining = false;
    let result = loop {
        let deadline = *deadlines.borrow_and_update();
        tokio::select! {
            biased;
            result = &mut driver => break result,
            () = shutdown.cancelled(), if !draining => {
                draining = true;
                driver.as_mut().graceful_shutdown();
            },
            changed = deadlines.changed() => {
                if changed.is_err() {
                    return;
                }
            },
            () = transport_deadline(deadline) => {
                // The budget also covers downstream JSON writes. Dropping the
                // driver stops a slow reader and signals the request token.
                tracing::warn!(phase = "http_connection", "HTTP connection deadline exceeded during response delivery or idle keepalive");
                return;
            },
        }
    };
    if let Err(source) = result {
        tracing::debug!(
            timeout = source.is_timeout(),
            incomplete = source.is_incomplete_message(),
            closed = source.is_closed(),
            "HTTP connection closed with transport failure"
        );
    }
}

async fn transport_deadline(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn shutdown_signal() -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            received = terminate.recv() => received.ok_or_else(||
                std::io::Error::other("termination signal stream closed")),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[cfg(test)]
mod tests;
