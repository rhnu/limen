//! Offline script verification through the production request and response pipeline.

mod schema;

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Request},
};
use limen_config::{ClientAuth, ResourceLimits};
use limen_script::Cancellation;
use schema::{Case, Headers};
use serde_json::{Value, json};

use crate::{
    bootstrap::Bootstrap,
    gateway::{FixtureResponse, Gateway, Trace},
};

const MAX_CASES: usize = 1024;

#[derive(thiserror::Error, Debug)]
pub(crate) enum Error {
    #[error("configuration failed")]
    Configuration(#[from] crate::bootstrap::Error),
    #[error("cannot read offline fixture")]
    Io(#[from] std::io::Error),
    #[error("invalid offline fixture JSON or schema")]
    Json(#[from] serde_json::Error),
    #[error("fixture exceeds JSON limits")]
    JsonLimits(#[source] limen_config::ConfigError),
    #[error("invalid fixture header name")]
    HeaderName(#[source] http::header::InvalidHeaderName),
    #[error("invalid bounded SSE output")]
    Stream(#[from] limen_chat::Error),
    #[error("invalid fixture header value")]
    HeaderValue(#[source] http::header::InvalidHeaderValue),
    #[error("fixture headers exceed capacity")]
    HeaderCapacity(#[source] http::header::MaxSizeReached),
    #[error("offline {phase} exceeded its deadline")]
    Deadline {
        phase: &'static str,
        #[source]
        source: tokio::time::error::Elapsed,
    },
    #[error("offline test failed and cleanup also failed")]
    Cleanup {
        #[source]
        primary: Box<Self>,
        cleanup: Box<Self>,
    },
    #[error("{0}")]
    Invalid(&'static str),
    #[error("cannot construct offline HTTP input")]
    Http(#[from] http::Error),
    #[error("cannot collect bounded offline output")]
    Body(#[from] axum::Error),
}

/// Mismatches contain only fixed category names, never actual or expected data.
pub(crate) struct Report {
    pub name: String,
    pub mismatches: Vec<&'static str>,
}

impl Report {
    pub fn passed(&self) -> bool { self.mismatches.is_empty() }
}

pub(crate) async fn run(config: Option<&Path>, cases: &Path) -> ExitCode {
    match run_all(config, cases).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            // Sources can contain fixture bodies, paths, credentials or queries.
            eprintln!("Offline tests failed: {error}");
            if let Error::Configuration(source) = error {
                crate::telemetry::report_failure(&source, "test", source.phase());
            }
            ExitCode::FAILURE
        },
    }
}

async fn run_all(config: Option<&Path>, cases: &Path) -> Result<bool, Error> {
    let (config, settings) = Bootstrap::settings(config)?;
    let paths = discover(cases)?;
    let mut passed = 0;
    for (index, path) in paths.iter().enumerate() {
        // Each case is a new service lifetime: no closure state leaks between cases.
        let loaded = Bootstrap::start(&config, settings.clone()).await?;
        let result = run_case(&loaded, path).await;
        let cleanup = loaded.shutdown().await;
        let report = match (result, cleanup) {
            (Ok(report), Ok(())) => report,
            (Err(primary), Err(cleanup)) => {
                return Err(Error::Cleanup {
                    primary: Box::new(primary),
                    cleanup: Box::new(Error::Configuration(cleanup)),
                });
            },
            (Err(error), Ok(())) => return Err(error),
            (Ok(_), Err(error)) => return Err(Error::Configuration(error)),
        };
        if report.passed() {
            passed += 1;
        }
        println!(
            "{} case {} ({}){}",
            if report.passed() { "PASS" } else { "FAIL" },
            index + 1,
            report.name,
            if report.passed() {
                String::new()
            } else {
                format!(": {}", report.mismatches.join(", "))
            }
        );
    }
    println!("{passed}/{} offline cases passed", paths.len());
    Ok(passed == paths.len())
}

pub(crate) fn discover(path: &Path) -> Result<Vec<PathBuf>, Error> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut paths = Vec::new();
    for entry in path.read_dir()? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".case.json"))
        {
            if paths.len() >= MAX_CASES {
                return Err(Error::Invalid("too many offline cases"));
            }
            paths.push(entry.path());
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(Error::Invalid("no .case.json files found"));
    }
    Ok(paths)
}

fn read(path: &Path, limit: usize) -> Result<Vec<u8>, Error> {
    if !path.metadata()?.is_file() {
        return Err(Error::Invalid("fixture must be a regular file"));
    }
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid("fixture must be a regular file"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Invalid("fixture exceeds byte limit"));
    }
    Ok(bytes)
}

pub(crate) async fn run_case(loaded: &Bootstrap, path: &Path) -> Result<Report, Error> {
    let limits = &loaded.server.limits;
    let bytes = read(path, limits.max_source_bytes)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    limen_config::validate_json(&value, limits.max_json_depth, limits.max_json_nodes)
        .map_err(Error::JsonLimits)?;
    let case: Case = serde_json::from_value(value)?;
    if !case.expect.has_assertion() {
        return Err(Error::Invalid(
            "case must declare at least one output assertion",
        ));
    }
    if case
        .expect
        .stats
        .as_ref()
        .is_some_and(|stats| !stats.valid_fields())
    {
        return Err(Error::Invalid("unknown statistics assertion field"));
    }
    if case.name.is_empty()
        || case.name.len() > 128
        || !case
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b))
    {
        return Err(Error::Invalid("case name must be a safe ASCII identifier"));
    }
    if !(100..600).contains(&case.upstream.status) {
        return Err(Error::Invalid("invalid fixture status"));
    }
    if case.upstream.body.is_some() && case.upstream.body_file.is_some() {
        return Err(Error::Invalid("choose body or bodyFile"));
    }
    let body = if let Some(relative) = &case.upstream.body_file {
        read(
            &path.parent().unwrap_or(Path::new(".")).join(relative),
            limits.max_aggregate_bytes.max(limits.max_output_bytes),
        )?
    } else if let Some(body) = &case.upstream.body {
        serde_json::to_vec(body)?
    } else {
        Vec::new()
    };
    let headers = header_map(&case.upstream.headers)?;
    let upstream = FixtureResponse {
        status: case.upstream.status,
        headers,
        body,
    };
    let request = client_request(&case, &loaded.server.client_auth)?;
    let (gateway, trace) = Gateway::offline(loaded, upstream);
    let budget = Duration::from_millis(limits.request_ms + limits.cleanup_ms + limits.shutdown_ms);
    let cancellation = Cancellation::new();
    let work = async {
        let response = Arc::clone(&gateway)
            .handle(request, cancellation.clone())
            .await;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let collected = to_bytes(
            response.into_body(),
            limits.max_aggregate_bytes.max(limits.max_output_bytes),
        )
        .await;
        gateway.drain().await;
        let bytes = collected?;
        let trace = trace.lock().await;
        compare(case, status, &headers, &bytes, &trace, limits)
    };
    let source = match tokio::time::timeout(budget, work).await {
        Ok(result) => return result,
        Err(source) => source,
    };
    cancellation.cancel();
    let primary = Error::Deadline {
        phase: "execution",
        source,
    };
    match tokio::time::timeout(Duration::from_millis(limits.shutdown_ms), gateway.drain()).await {
        Ok(()) => Err(primary),
        Err(source) => Err(Error::Cleanup {
            primary: Box::new(primary),
            cleanup: Box::new(Error::Deadline {
                phase: "cleanup",
                source,
            }),
        }),
    }
}

/// Build the same client input shape without allowing fixtures to replace runner credentials.
fn client_request(case: &Case, auth: &ClientAuth) -> Result<Request<Body>, Error> {
    let client_headers = header_map(&case.client_headers)?;
    if client_headers.contains_key("authorization") || client_headers.contains_key("content-type") {
        return Err(Error::Invalid(
            "fixture client authentication and media type are managed by the runner",
        ));
    }
    let mut request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("content-type", "application/json");
    if let ClientAuth::Bearer(token) = auth {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    for (name, value) in &client_headers {
        request = request.header(name, value);
    }
    let request = request.body(Body::from(serde_json::to_vec(&case.request)?))?;
    let header_bytes = request
        .headers()
        .iter()
        .try_fold(2_usize, |total, (name, value)| {
            total
                .checked_add(name.as_str().len())?
                .checked_add(value.as_bytes().len())?
                .checked_add(4)
        });
    if request.headers().len() > 100 || header_bytes.is_none_or(|bytes| bytes > 32 * 1024) {
        return Err(Error::Invalid(
            "fixture client headers exceed HTTP input limits",
        ));
    }
    Ok(request)
}

fn header_map(input: &Headers) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    for (name, values) in input {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(Error::HeaderName)?;
        if values.is_empty() {
            return Err(Error::Invalid("fixture header values must not be empty"));
        }
        if headers.contains_key(&name) {
            return Err(Error::Invalid("duplicate fixture header name"));
        }
        for value in values {
            let value = HeaderValue::from_str(value).map_err(Error::HeaderValue)?;
            headers
                .try_append(name.clone(), value)
                .map_err(Error::HeaderCapacity)?;
        }
    }
    Ok(headers)
}

fn compare(
    case: Case,
    status: u16,
    response_headers: &HeaderMap,
    bytes: &[u8],
    trace: &Trace,
    limits: &ResourceLimits,
) -> Result<Report, Error> {
    let mut report = Report {
        name: case.name,
        mismatches: Vec::new(),
    };
    let expected = case.expect;
    let mut check = |category, matches: bool| {
        if !matches {
            report.mismatches.push(category);
        }
    };
    if let Some(sent) = expected.sent {
        check("sent", trace.sent.is_some() == sent);
    }
    if let Some(outbound) = expected.outbound {
        if let Some(body) = outbound.body {
            check(
                "outbound.body",
                trace.sent.as_ref().is_some_and(|sent| sent.body == body),
            );
        }
        if let Some(url) = outbound.url {
            check(
                "outbound.url",
                trace.sent.as_ref().is_some_and(|sent| sent.url == url),
            );
        }
        if let Some(fields) = outbound.headers {
            let fields = header_map(&fields)?;
            check(
                "outbound.headers",
                trace
                    .sent
                    .as_ref()
                    .is_some_and(|sent| sent.headers == fields),
            );
        }
    }
    if let Some(response) = expected.response {
        if let Some(expected) = response.status {
            check("response.status", status == expected);
        }
        if let Some(fields) = response.headers {
            check(
                "response.headers",
                response_headers == &header_map(&fields)?,
            );
        }
        if let Some(body) = response.body {
            check(
                "response.body",
                serde_json::from_slice::<Value>(bytes).is_ok_and(|actual| actual == body),
            );
        }
        if response.events.is_some() || response.done.is_some() {
            let actual = if response_headers
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|media| crate::gateway::media_kind(media) == "sse")
            {
                stream(bytes, limits)
            } else {
                Err(Error::Invalid("SSE assertions require an SSE response"))
            };
            if let Some(expected) = response.events {
                check(
                    "response.events",
                    actual.as_ref().is_ok_and(|(events, _)| events == &expected),
                );
            }
            if let Some(expected) = response.done {
                check(
                    "response.done",
                    actual.as_ref().is_ok_and(|(_, done)| done == &expected),
                );
            }
        }
    }
    if let Some(state) = expected.state {
        check("state", state == trace.state);
    }
    if let Some(stats) = expected.stats {
        compare_statistics(stats, &trace.stats, &mut check);
    }
    if let Some(observations) = expected.observations {
        check("observations", trace.observations == observations);
    }
    compare_terminal(
        expected.error,
        expected.observation_failed,
        trace,
        &mut check,
    );
    Ok(report)
}

fn compare_terminal(
    error: Option<schema::Error>,
    observation_failed: Option<bool>,
    trace: &Trace,
    check: &mut impl FnMut(&'static str, bool),
) {
    if let Some(error) = error {
        check(
            "error",
            trace.error
                == Some(json!({"code":error.code,"status":error.status,"phase":error.phase})),
        );
    } else {
        check("error", trace.error.is_none());
    }
    check(
        "observationFailed",
        trace.observation_failed == observation_failed.unwrap_or(false),
    );
}

fn compare_statistics(
    expected: schema::ExpectedStats,
    actual: &Value,
    check: &mut impl FnMut(&'static str, bool),
) {
    if let Some(usage) = expected.usage {
        check("stats.usage", actual["usage"] == usage);
    }
    if !expected.hook_calls.is_empty() {
        check(
            "stats.hookCalls",
            expected.hook_calls.iter().all(|(phase, count)| {
                actual["timings"]["hooks"][phase]["calls"]
                    .as_u64()
                    .unwrap_or(0)
                    == *count
            }),
        );
    }
    if !expected.timing_presence.is_empty() {
        check(
            "stats.timingPresence",
            expected
                .timing_presence
                .iter()
                .all(|(field, present)| actual["timings"][field].is_number() == *present),
        );
    }
}

fn stream(bytes: &[u8], limits: &ResourceLimits) -> Result<(Vec<Value>, bool), Error> {
    let mut parser = limen_chat::FrameDecoder::new(limits.max_event_bytes.max(1024));
    let mut events = Vec::new();
    let mut done = false;
    for byte in bytes {
        let frame = parser.push(*byte).map_err(|source| {
            if matches!(source, limen_chat::Error::EventLimit) {
                Error::Invalid("offline SSE event exceeds byte limit")
            } else {
                Error::Stream(source)
            }
        })?;
        if let Some(frame) = frame {
            let event = match frame.event() {
                Err(limen_chat::Error::UpstreamError) => continue,
                result => result?,
            };
            let value = match event {
                limen_chat::Event::KeepAlive => continue,
                limen_chat::Event::Done => {
                    done = true;
                    continue;
                },
                limen_chat::Event::Chunk(value) => value,
            };
            if frame.event == "error" {
                continue;
            }
            if events.len() >= limits.max_aggregate_events {
                return Err(Error::Invalid("offline SSE exceeds event limit"));
            }
            events.push(value);
        }
    }
    Ok((events, done))
}

#[cfg(test)]
mod tests;
