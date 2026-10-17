use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{Cancellation, RuntimeOptions, RuntimeStatus, ScriptRuntime};

mod context;
mod contract;
mod diagnostics;
mod observations;
mod policies;
mod runtime;
mod timings;

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

impl FixtureReport {
    fn into_context(self) -> Result<Value, crate::ScriptError> {
        self.outcome.map(|output| output.context)
    }
}

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error + Send + Sync>> {
        Ok(Self {
            root: tempfile::tempdir()?,
        })
    }

    fn write(&self, relative: &str, source: &str) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
        let path = self.root.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, source)?;
        Ok(path)
    }

    async fn start(
        &self,
        source: &str,
    ) -> Result<(ScriptRuntime, Value), Box<dyn Error + Send + Sync>> {
        let path = self.write("config.js", source)?;
        let started = ScriptRuntime::start(path, RuntimeOptions::default()).await?;
        ensure(
            matches!(started.0.status(), RuntimeStatus::Ready),
            "successful startup did not publish Ready",
        )?;
        Ok(started)
    }
}

fn hook(config: &Value, name: &str) -> Result<u64, Box<dyn Error + Send + Sync>> {
    config[name]
        .as_u64()
        .ok_or_else(|| format!("missing registered {name}").into())
}

// Only fixtures use arbitrary metadata knobs. Permission comes from the explicit phase.
struct FixtureCall {
    hook_id: u64,
    context: Value,
    phase: crate::HookPhase,
    raw_state: bool,
    deadline: Instant,
    cancellation: Cancellation,
}

struct FixtureOutput {
    context: Value,
    observations: crate::ObservationBatch,
}

struct FixtureReport {
    outcome: Result<FixtureOutput, crate::ScriptError>,
    timings: crate::HookTimings,
}

fn fixture_phase(phase: &str) -> crate::HookPhase {
    use crate::HookPhase as P;
    match phase {
        "onRequest" => P::OnRequest,
        "beforeSend" => P::BeforeSend,
        "afterReceive" => P::AfterReceive,
        "onResponse" => P::OnResponse,
        "onContent" => P::OnContent,
        "onUpstreamEvent" => P::OnUpstreamEvent,
        "onStreamEvent" => P::OnStreamEvent,
        "onErrorResponse" => P::OnErrorResponse,
        "onError" => P::OnError,
        "onFinish" => P::OnFinish,
        _ => panic!("unknown fixture phase"),
    }
}

fn call(hook_id: u64, _call_id: u64, context: Value) -> FixtureCall {
    let raw_state = context.get("meta").is_none();
    let phase = context["meta"]["phase"]
        .as_str()
        .map_or(crate::HookPhase::OnRequest, fixture_phase);
    let context = if raw_state
        && !["reject", "rejection", "observe", "observations"]
            .iter()
            .any(|key| context.get(key).is_some())
    {
        json!({"meta":{"phase":"onRequest"},"state":context,"body":{}})
    } else {
        context
    };
    FixtureCall {
        hook_id,
        context,
        phase,
        raw_state,
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: Cancellation::new(),
    }
}

impl ScriptRuntime {
    async fn invoke_fixture(&self, call: FixtureCall) -> FixtureReport {
        let started = Instant::now();
        let bytes = crate::json::encode_owned(&call.context, self.limits);
        let input_bridge = started.elapsed();
        let mut report = match bytes {
            Ok(bytes) => {
                self.invoke_encoded(
                    call.hook_id,
                    call.phase,
                    bytes,
                    call.deadline,
                    call.cancellation,
                )
                .await
            },
            Err(error) => crate::HookReport::failed(error),
        };
        report.timings.input_bridge += input_bridge;
        report.timings.wall = started.elapsed();
        FixtureReport {
            outcome: report.outcome.map(|output| {
                let mut context = fixture_output(&output);
                if call.raw_state {
                    context = context["state"].take();
                }
                FixtureOutput {
                    context,
                    observations: output.effects.observations,
                }
            }),
            timings: report.timings,
        }
    }
}

fn fixture_output(output: &crate::HookOutput) -> Value {
    use crate::HookCandidate as C;
    let mut value = json!({"state":output.effects.state});
    let rejection = match &output.candidate {
        C::OnRequest { body, rejection } => {
            value["body"] = body.clone();
            *rejection
        },
        C::BeforeSend { request, rejection } => {
            value["body"] = request.body.clone();
            value["url"] = json!(request.url);
            value["headers"] = crate::contract::headers_json(&request.headers);
            *rejection
        },
        C::AfterReceive(head) => {
            value["status"] = json!(head.status);
            value["headers"] = crate::contract::headers_json(&head.headers);
            None
        },
        C::OnResponse { body } => {
            value["body"] = body.clone();
            None
        },
        C::OnContent { events } | C::OnUpstreamEvent { events } | C::OnStreamEvent { events } => {
            value["events"] = json!(events);
            None
        },
        C::OnErrorResponse(draft) => {
            value["body"] = json!(draft.body);
            value["contentType"] = json!(draft.content_type);
            value["status"] = json!(draft.status);
            value["headers"] = crate::contract::headers_json(&draft.headers);
            None
        },
        C::OnError | C::OnFinish => None,
    };
    if let Some(status) = rejection {
        value["rejection"] = json!({"status":status.as_u16()});
    }
    value
}

fn ensure(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

async fn stop(runtime: &ScriptRuntime) -> TestResult {
    ensure(
        matches!(runtime.shutdown().await, RuntimeStatus::Stopped),
        "ordinary VM shutdown failed",
    )
}

async fn wait_for(runtime: &ScriptRuntime, id: u64, expected: Value) -> TestResult {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if runtime
                .invoke_fixture(call(id, 1000, json!({})))
                .await
                .into_context()?
                == expected
            {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await?
}
