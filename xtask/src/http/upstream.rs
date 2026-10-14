//! Hyper owns HTTP framing; this fixture records requests and controls synthetic responses.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use http::{HeaderMap, Response};
use http_body_util::{BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Mutex, watch},
    task::{JoinHandle, JoinSet},
    time::{Instant, sleep},
};

#[derive(Clone)]
pub struct Record {
    pub path: String,
    pub headers: HeaderMap,
    pub body: Value,
}

pub struct Controls {
    pub records: Vec<Record>,
    pub started: bool,
    pub disconnected: bool,
    pub status: u16,
    pub media: &'static str,
    pub response: Option<Value>,
    pub raw: Option<Vec<u8>>,
    pub delay: bool,
    errors: Vec<anyhow::Error>,
}

impl Default for Controls {
    /// Start with a JSON echo response and no artificial delay.
    fn default() -> Self {
        Self {
            records: Vec::new(),
            started: false,
            disconnected: false,
            status: 200,
            media: "application/json",
            response: None,
            raw: None,
            delay: false,
            errors: Vec::new(),
        }
    }
}

pub struct Upstream {
    pub port: u16,
    pub controls: Arc<Mutex<Controls>>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
}

impl Upstream {
    /// Bind only loopback and own every accepted connection in one task tree.
    pub async fn start() -> Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("bind synthetic upstream")?;
        let port = listener
            .local_addr()
            .context("read synthetic upstream address")?
            .port();
        let controls = Arc::new(Mutex::new(Controls::default()));
        let shared = Arc::clone(&controls);
        let (shutdown, mut stopping) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    changed = stopping.changed() => {
                        changed.context("synthetic upstream stop channel")?;
                        break;
                    },
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.context("accept synthetic upstream connection")?;
                        let state = Arc::clone(&shared);
                        connections.spawn(async move {
                            let delayed = Arc::new(Mutex::new(false));
                            let request_state = Arc::clone(&state);
                            let request_delayed = Arc::clone(&delayed);
                            let service = service_fn(move |request| {
                                respond(request, Arc::clone(&request_state), Arc::clone(&request_delayed))
                            });
                            let result = http1::Builder::new().half_close(false)
                                .serve_connection(TokioIo::new(stream), service).await;
                            if *delayed.lock().await {
                                // Hyper detects EOF while the response future is pending.
                                state.lock().await.disconnected = true;
                            } else if let Err(error) = result {
                                state.lock().await.errors.push(anyhow::Error::new(error)
                                    .context("serve synthetic upstream connection"));
                            }
                        });
                    },
                    joined = connections.join_next(), if !connections.is_empty() => {
                        if let Some(result) = joined {
                            result.context("synthetic upstream connection task")?;
                        }
                    },
                }
            }
            drop(listener);
            connections.abort_all();
            while let Some(joined) = connections.join_next().await {
                match joined {
                    Err(error) if error.is_cancelled() => {},
                    Err(error) => {
                        return Err(error).context("stop synthetic upstream connection task");
                    },
                    Ok(()) => {},
                }
            }
            Ok(())
        });
        Ok(Self {
            port,
            controls,
            shutdown,
            task,
        })
    }

    /// Wait for a recorded upstream lifecycle event within its assertion budget.
    pub async fn wait(&self, timeout: Duration, disconnected: bool) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let state = self.controls.lock().await;
            if if disconnected {
                state.disconnected
            } else {
                state.started
            } {
                return Ok(());
            }
            drop(state);
            ensure!(
                Instant::now() < deadline,
                "synthetic upstream {} not observed",
                if disconnected { "disconnect" } else { "start" }
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Stop the server and report background failures before its tasks are released.
    pub async fn stop(&mut self) -> Result<()> {
        self.shutdown.send_replace(true);
        (&mut self.task)
            .await
            .context("synthetic upstream server task")??;
        let mut state = self.controls.lock().await;
        if let Some(error) = state.errors.pop() {
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for Upstream {
    /// Abort the listener task; its `JoinSet` aborts all owned connection tasks.
    fn drop(&mut self) { self.task.abort(); }
}

/// Capture multivalue headers unchanged and return controlled JSON or SSE bytes.
async fn respond(
    request: hyper::Request<Incoming>,
    state: Arc<Mutex<Controls>>,
    delayed: Arc<Mutex<bool>>,
) -> Result<Response<Full<Bytes>>> {
    let (parts, body) = request.into_parts();
    let raw = body
        .collect()
        .await
        .context("read synthetic upstream request")?
        .to_bytes();
    let body: Value =
        serde_json::from_slice(&raw).context("decode synthetic upstream request JSON")?;
    let mut controls = state.lock().await;
    controls.records.push(Record {
        path: parts.uri.to_string(),
        headers: parts.headers,
        body: body.clone(),
    });
    controls.started = true;
    if controls.delay {
        *delayed.lock().await = true;
        drop(controls);
        // This future is dropped by Hyper on remote EOF or by fixture cleanup.
        return std::future::pending().await;
    }
    let response = controls.response.clone().unwrap_or_else(
        || json!({"id":"synthetic", "model":"real-model", "choices":[], "echo":body}),
    );
    let raw = match &controls.raw {
        Some(raw) => raw.clone(),
        None => serde_json::to_vec(&response).context("encode synthetic upstream response")?,
    };
    let mut builder = Response::builder()
        .status(controls.status)
        .header("Content-Type", controls.media)
        .header("Content-Length", raw.len())
        .header("Set-Cookie", "synthetic-cookie-secret=1");
    if controls.status == 307 {
        builder = builder.header("Location", "/redirect-target");
    }
    builder
        .body(Full::new(Bytes::from(raw)))
        .context("construct synthetic upstream response")
}
