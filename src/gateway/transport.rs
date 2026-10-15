//! The single upstream boundary, shared by live requests and offline fixtures.

use std::{collections::BTreeSet, sync::Arc};

use http::HeaderMap;
use serde_json::Value;
use tokio::sync::Mutex;

use super::{
    error::{ErrorCode, Failure},
    logic,
};

/// Actual HTTP facts and raw body bytes, including SSE framing.
pub(crate) struct FixtureResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub(crate) struct SentRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Value,
}

#[derive(Default)]
pub(crate) struct Trace {
    pub sent: Option<SentRequest>,
    pub state: Value,
    pub error: Option<Value>,
    pub observation_failed: bool,
}

pub(super) enum Transport {
    Http {
        client: reqwest::Client,
        origins: BTreeSet<String>,
    },
    Fixture {
        response: Mutex<Option<FixtureResponse>>,
        trace: Arc<Mutex<Trace>>,
    },
}

impl Transport {
    pub fn http(origins: BTreeSet<String>) -> Result<Self, reqwest::Error> {
        Ok(Self::Http {
            client: super::upstream_client(1)?,
            origins,
        })
    }

    pub fn fixture(response: FixtureResponse) -> (Self, Arc<Mutex<Trace>>) {
        let trace = Arc::new(Mutex::new(Trace::default()));
        (
            Self::Fixture {
                response: Mutex::new(Some(response)),
                trace: Arc::clone(&trace),
            },
            trace,
        )
    }

    pub async fn send(
        &self,
        url: String,
        headers: HeaderMap,
        body: Vec<u8>,
    ) -> Result<Incoming, Failure> {
        match self {
            Self::Http { client, origins } => {
                let client = if origins.contains(&logic::safe_target(&url)) {
                    client.clone()
                } else {
                    super::upstream_client(0)
                        .map_err(|e| Failure::upstream(e, "upstream_client"))?
                };
                client
                    .post(url)
                    .headers(headers)
                    .body(body)
                    .send()
                    .await
                    .map(Incoming::Http)
                    .map_err(|e| Failure::upstream(e, "upstream_send"))
            },
            Self::Fixture { response, trace } => {
                let response = response
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| Failure::new(ErrorCode::Internal, "offline_send"))?;
                let body = serde_json::from_slice(&body).map_err(|e| {
                    Failure::new(ErrorCode::Internal, "offline_send").with_source(e)
                })?;
                trace.lock().await.sent = Some(SentRequest { url, headers, body });
                Ok(Incoming::Fixture {
                    response,
                    read: false,
                })
            },
        }
    }

    pub async fn complete(&self, state: &Value, error: Option<Value>) {
        if let Self::Fixture { trace, .. } = self {
            let mut trace = trace.lock().await;
            trace.state = state.clone();
            trace.error = error;
        }
    }

    pub async fn observation_failed(&self) {
        if let Self::Fixture { trace, .. } = self {
            trace.lock().await.observation_failed = true;
        }
    }
}

pub(super) enum Incoming {
    Http(reqwest::Response),
    Fixture {
        response: FixtureResponse,
        read: bool,
    },
}

impl Incoming {
    pub fn status(&self) -> u16 {
        match self {
            Self::Http(r) => r.status().as_u16(),
            Self::Fixture { response, .. } => response.status,
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        match self {
            Self::Http(r) => r.headers(),
            Self::Fixture { response, .. } => &response.headers,
        }
    }

    pub async fn chunk(&mut self) -> Result<Option<axum::body::Bytes>, Failure> {
        match self {
            Self::Http(r) => r
                .chunk()
                .await
                .map_err(|e| Failure::upstream(e, "upstream_receive")),
            Self::Fixture { response, read } => {
                if *read {
                    return Ok(None);
                }
                *read = true;
                Ok(Some(std::mem::take(&mut response.body).into()))
            },
        }
    }
}
