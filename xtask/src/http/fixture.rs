//! Gateway fixtures use distinct directories under the supervisor-owned temporary root.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use http::{HeaderMap, Method};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{Instant, sleep, timeout},
};

use super::upstream::Upstream;
use crate::process::Gateway;

const DEFAULT_CONFIG: &str = r"export default {
 providers: {p: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'},
 defaults: {temperature: 0.2, extra: {provider: true}}}},
 models: {chat: {provider: 'p', upstream: 'real-model', defaults: {temperature: 0.4}}}
};";

pub const PENDING_CONFIG: &str = r"let cancelled = 0, finishes = 0;
export default {onRequest(ctx){if(ctx.body.wait)return new Promise(()=>{});},
 onFinish(ctx){finishes++;if(ctx.outcome==='cancelled')cancelled++;},
 providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'report',onResponse(ctx){ctx.body.cancelled=cancelled;ctx.body.finishes=finishes;}}]}},
 models:{chat:{provider:'p',upstream:'real-model'}}};";

#[derive(Default)]
pub struct Options<'a> {
    pub config: Option<&'a str>,
    pub limits: &'a str,
    pub no_auth: bool,
}

pub struct Laboratory<'a> {
    executable: &'a Path,
    directory: &'a Path,
    logs: Vec<PathBuf>,
}

impl<'a> Laboratory<'a> {
    /// Keep fixture files beneath the directory held by the external supervisor.
    pub fn new(executable: &'a Path, directory: &'a Path) -> Self {
        Self {
            executable,
            directory,
            logs: Vec::new(),
        }
    }

    /// Complete fixture cleanup even when an assertion or transport operation fails.
    pub async fn with_fixture<F>(&mut self, options: Options<'_>, action: F) -> Result<()>
    where
        F: AsyncFnOnce(&mut Fixture) -> Result<()>,
    {
        let directory = self.directory.join(format!("fixture-{}", self.logs.len()));
        fs::create_dir(&directory).context("create HTTP fixture directory")?;
        let log_path = directory.join("process.log");
        self.logs.push(log_path.clone());
        let mut fixture = Fixture::start(self.executable, &directory, &log_path, options).await?;
        let result = action(&mut fixture).await;
        drop(fixture.process);
        let cleanup = fixture.upstream.stop().await;
        match (result, cleanup) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(primary), Err(cleanup)) => {
                Err(primary.context(format!("upstream cleanup also failed: {cleanup:#}")))
            },
        }
    }

    /// Verify all placeholder credentials, bodies, cookies, and query values stay out of logs.
    pub fn check_logs(&self) -> Result<()> {
        for path in &self.logs {
            let text = fs::read_to_string(path)
                .with_context(|| format!("read HTTP fixture log {}", path.display()))?;
            for secret in [
                "synthetic-client-secret",
                "synthetic-upstream-secret",
                "synthetic-query-secret",
                "synthetic-body-secret",
                "synthetic-cookie-secret",
            ] {
                ensure!(
                    !text.contains(secret),
                    "process log exposed synthetic secret"
                );
            }
        }
        Ok(())
    }
}

pub struct Fixture {
    pub upstream: Upstream,
    pub process: Gateway,
    port: u16,
    log_path: PathBuf,
    client: reqwest::Client,
}

pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Reply {
    /// Read a required response header using HTTP's case-insensitive lookup.
    pub fn header(&self, name: &str) -> Result<&str> {
        self.headers
            .get(name)
            .context("required response header missing")?
            .to_str()
            .context("required response header is not text")
    }

    /// Access SSE text while JSON replies retain their structured representation.
    pub fn text(&self) -> Result<&str> { self.body.as_str().context("expected SSE response text") }

    /// Assert the public status and safe error code together.
    pub fn error(&self, status: u16, code: &str, meaning: &str) -> Result<()> {
        ensure!(
            self.status == status && self.body["error"]["code"] == code,
            "{meaning}"
        );
        Ok(())
    }
}

impl Fixture {
    /// Write synthetic configuration, start the supervised gateway, and await its listener.
    async fn start(
        executable: &Path,
        directory: &Path,
        log_path: &Path,
        options: Options<'_>,
    ) -> Result<Self> {
        let upstream = Upstream::start().await?;
        let reserve = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .context("reserve gateway loopback port")?;
        let port = reserve
            .local_addr()
            .context("read reserved gateway port")?
            .port();
        drop(reserve);
        let endpoint = format!(
            "http://127.0.0.1:{}/custom/completion?key=synthetic-query-secret",
            upstream.port
        );
        let config = options.config.unwrap_or(DEFAULT_CONFIG).replace(
            "ENDPOINT",
            &serde_json::to_string(&endpoint).context("encode synthetic endpoint")?,
        );
        fs::write(directory.join("config.js"), config).context("write synthetic HTTP script")?;
        let auth = if options.no_auth {
            "none: true"
        } else {
            "bearer: synthetic-client-secret"
        };
        let yaml = format!(
            "listen: 127.0.0.1:{port}\nclient_auth: {{{auth}}}\nlog: {{level: debug, format: json}}\nlimits:\n  request_ms: 1500\n  hook_ms: 1000\n  cleanup_ms: 200\n  shutdown_ms: 1500\n  concurrency: 2\n  queue_capacity: 2\n{}",
            options.limits
        );
        let config_path = directory.join("server.yaml");
        fs::write(&config_path, yaml).context("write synthetic HTTP server configuration")?;
        let log = File::create(log_path).context("create HTTP fixture log")?;
        let process = Gateway::spawn(executable, &config_path, &log)?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(4))
            .build()
            .context("create HTTP experiment client")?;
        let mut fixture = Self {
            upstream,
            process,
            port,
            log_path: log_path.to_owned(),
            client,
        };
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            ensure!(
                fixture
                    .process
                    .try_wait()
                    .context("poll gateway startup")?
                    .is_none(),
                "gateway failed during startup"
            );
            if let Ok(Ok(stream)) = timeout(
                Duration::from_millis(100),
                TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
            )
            .await
            {
                drop(stream);
                return Ok(fixture);
            }
            ensure!(Instant::now() < deadline, "gateway did not listen");
            sleep(Duration::from_millis(20)).await;
        }
    }

    /// Send an ordinary authenticated JSON generation request.
    pub async fn request(&self, body: Value) -> Result<Reply> {
        self.send(body, Method::POST, "/v1/chat/completions", &[])
            .await
    }

    /// Override or remove fixture headers without enabling redirects or environment proxies.
    pub async fn send(
        &self,
        body: Value,
        method: Method,
        path: &str,
        headers: &[(&str, Option<&str>)],
    ) -> Result<Reply> {
        let mut fields = HeaderMap::new();
        fields.insert(
            "authorization",
            http::HeaderValue::from_static("Bearer synthetic-client-secret"),
        );
        fields.insert(
            "content-type",
            http::HeaderValue::from_static("application/json"),
        );
        fields.insert("connection", http::HeaderValue::from_static("close"));
        for (name, value) in headers {
            let name = http::HeaderName::from_bytes(name.as_bytes())
                .context("parse experiment header name")?;
            if let Some(value) = value {
                fields.insert(
                    name,
                    http::HeaderValue::from_str(value).context("parse experiment header value")?,
                );
            } else {
                fields.remove(name);
            }
        }
        let mut request = self
            .client
            .request(
                method.clone(),
                format!("http://127.0.0.1:{}{path}", self.port),
            )
            .headers(fields);
        if method == Method::POST {
            request = request.body(serde_json::to_vec(&body).context("encode experiment request")?);
        }
        let response = request
            .send()
            .await
            .context("send HTTP experiment request")?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let sse = headers
            .get("content-type")
            .map(http::HeaderValue::to_str)
            .transpose()
            .context("decode HTTP experiment response content type")?
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/event-stream"))
            });
        let raw = response
            .bytes()
            .await
            .context("read HTTP experiment response")?;
        let body = if sse {
            Value::String(
                String::from_utf8(raw.to_vec()).context("decode SSE experiment response")?,
            )
        } else {
            serde_json::from_slice(&raw).context("decode JSON experiment response")?
        };
        Ok(Reply {
            status,
            headers,
            body,
        })
    }

    /// Confirm admission with 100 Continue before sending the body and retaining the socket.
    pub async fn open_request(&self, body: Value) -> Result<TcpStream> {
        let raw = serde_json::to_vec(&body).context("encode held experiment request")?;
        let mut stream = timeout(
            Duration::from_secs(2),
            TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, self.port)),
        )
        .await
        .context("held request connection deadline")?
        .context("connect held experiment request")?;
        let header = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer synthetic-client-secret\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nContent-Length: {}\r\n\r\n",
            raw.len()
        );
        timeout(Duration::from_secs(2), async {
            stream.write_all(header.as_bytes()).await?;
            // Hyper requests the body only after the gateway acquires a permit.
            // A completed socket write alone does not establish request admission.
            let mut interim = Vec::new();
            while !interim.ends_with(b"\r\n\r\n") {
                ensure!(
                    interim.len() < 32 * 1024,
                    "held request interim headers exceeded limit"
                );
                interim.push(
                    stream
                        .read_u8()
                        .await
                        .context("read held request admission response")?,
                );
            }
            ensure!(
                interim.starts_with(b"HTTP/1.1 100 "),
                "held request was rejected before body admission"
            );
            stream
                .write_all(&raw)
                .await
                .context("write admitted request body")
        })
        .await
        .context("held request admission deadline")?
        .context("admit held experiment request")?;
        Ok(stream)
    }

    /// Read the gateway's JSON log without parsing incomplete trailing lines.
    pub fn logs(&self) -> Result<String> {
        fs::read_to_string(&self.log_path).context("read gateway experiment log")
    }

    /// Extract complete structured log events and propagate malformed complete lines.
    pub fn events(&self) -> Result<Vec<Value>> {
        self.logs()?
            .split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| {
                serde_json::from_str::<Value>(line)
                    .map(|event| event["fields"].clone())
                    .context("decode gateway JSON log event")
            })
            .collect()
    }

    /// Wait for terminal observation diagnostics without racing asynchronous cleanup.
    pub async fn wait_event(&self, predicate: impl Fn(&Value) -> bool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if self.events()?.iter().any(&predicate) {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "expected bounded observation did not finish"
            );
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Poll until disconnected requests have released their admission permits.
    pub async fn wait_cancelled(&self, count: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_millis(800);
        loop {
            let response = self.request(request(&[])).await?;
            if response.status == 200 && response.body["cancelled"] == count {
                return Ok(());
            }
            ensure!(
                Instant::now() < deadline,
                "disconnect did not cancel pending hooks and release admission permits"
            );
            sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Apply scenario-specific fields to the same minimal placeholder-secret request.
pub fn request(fields: &[(&str, Value)]) -> Value {
    let mut body =
        json!({"model":"chat", "messages":[{"role":"user", "content":"synthetic-body-secret"}]});
    for (key, value) in fields {
        body[*key] = value.clone();
    }
    body
}
