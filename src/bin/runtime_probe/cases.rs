use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use limen_script::{
    Cancellation, HookCall, RuntimeOptions, RuntimeStatus, ScriptErrorKind, ScriptRuntime,
};
use serde_json::{Value, json};

#[path = "cases/context.rs"]
mod context;

type ProbeResult = Result<(), Box<dyn Error>>;

/// The external parent owns the root; this guard removes only files it created.
struct Fixture {
    root: PathBuf,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

impl Fixture {
    fn new(root: PathBuf) -> Result<Self, Box<dyn Error>> {
        if !root.is_dir() || fs::read_dir(&root)?.next().is_some() {
            return Err("fixture directory must exist and be empty".into());
        }
        Ok(Self {
            root,
            files: Vec::new(),
            directories: Vec::new(),
        })
    }

    fn write(&mut self, relative: &str, source: &str) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent().filter(|parent| !parent.exists()) {
            fs::create_dir(parent)?;
            self.directories.push(parent.to_path_buf());
        }
        fs::write(&path, source)?;
        if !self.files.contains(&path) {
            self.files.push(path.clone());
        }
        Ok(path)
    }

    async fn start(&mut self, source: &str) -> Result<(ScriptRuntime, Value), Box<dyn Error>> {
        let path = self.write("config.js", source)?;
        Ok(ScriptRuntime::start(path, RuntimeOptions::default()).await?)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in self.files.iter().rev() {
            drop(fs::remove_file(path));
        }
        for path in self.directories.iter().rev() {
            drop(fs::remove_dir(path));
        }
    }
}

pub async fn run(case: &str, root: PathBuf) -> ProbeResult {
    let mut fixture = Fixture::new(root)?;
    match case {
        "void" => void(&mut fixture).await,
        "pending-fairness" => pending_fairness(&mut fixture).await,
        "late-completion" => late_completion(&mut fixture).await,
        "invalid-return" => invalid_return(&mut fixture).await,
        "rejection" => rejection(&mut fixture).await,
        "top-level-await" => top_level_await(&mut fixture).await,
        "strict-json" => strict_json(&mut fixture).await,
        "modules" => modules(&mut fixture).await,
        "sync-loop" | "microtask-loop" | "memory-limit" => retirement(&mut fixture, case).await,
        "shutdown-pending" => shutdown_pending(&mut fixture).await,
        "startup-loop" => startup_loop(&mut fixture).await,
        "context-headers" => context::headers(&mut fixture).await,
        "context-readonly" => context::readonly(&mut fixture).await,
        "context-isolation" => context::isolation(&mut fixture).await,
        _ => Err("unknown experiment".into()),
    }
}

fn hook(config: &Value, name: &str) -> Result<u64, Box<dyn Error>> {
    config[name]
        .as_u64()
        .ok_or_else(|| format!("missing registered {name}").into())
}

fn call(hook_id: u64, call_id: u64, context: Value) -> HookCall {
    HookCall {
        request_id: format!("synthetic-{call_id}"),
        call_id,
        hook_id,
        context,
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: Cancellation::new(),
    }
}

fn ensure(condition: bool, message: &str) -> ProbeResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

async fn stop(runtime: &ScriptRuntime) -> ProbeResult {
    ensure(
        matches!(runtime.shutdown().await, RuntimeStatus::Stopped),
        "ordinary VM shutdown failed",
    )
}

async fn void(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { ctx.sync = true; }, async onFinish(ctx) { await Promise.resolve(); ctx.async = true; }, providers: { p: { pipeline: [{ name: 'provider', beforeSend(ctx) { ctx.order.push('provider'); } }] } }, models: { m: { pipeline: [{ name: 'model', async beforeSend(ctx) { await Promise.resolve(); ctx.order.push('model'); } }] } } };",
    ).await?;
    let sync = runtime
        .invoke(call(hook(&config, "onRequest")?, 1, json!({})))
        .await?;
    ensure(
        sync == json!({"sync": true}),
        "sync void draft did not commit",
    )?;
    let asynchronous = runtime
        .invoke(call(hook(&config, "onFinish")?, 2, json!({})))
        .await?;
    ensure(
        asynchronous == json!({"async": true}),
        "async void draft did not commit",
    )?;
    let provider = config["providers"]["p"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("provider hook not registered")?;
    let model = config["models"]["m"]["pipeline"][0]["beforeSend"]
        .as_u64()
        .ok_or("model hook not registered")?;
    let provider_output = runtime
        .invoke(call(provider, 3, json!({"order": []})))
        .await?;
    let model_output = runtime.invoke(call(model, 4, provider_output)).await?;
    ensure(
        model_output == json!({"order": ["provider", "model"]}),
        "saved provider/model functions did not preserve sequential input",
    )?;
    stop(&runtime).await
}

async fn pending_fairness(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { if (ctx.pending) return new Promise(() => {}); ctx.done = true; } };",
    ).await?;
    let id = hook(&config, "onRequest")?;
    for deadline in [false, true] {
        let mut pending = call(id, if deadline { 3 } else { 1 }, json!({"pending": true}));
        if deadline {
            pending.deadline = Instant::now() + Duration::from_millis(80);
        }
        let cancellation = pending.cancellation.clone();
        let pending_runtime = runtime.clone();
        let waiting = tokio::spawn(async move { pending_runtime.invoke(pending).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let output = runtime
            .invoke(call(id, if deadline { 4 } else { 2 }, json!({})))
            .await?;
        ensure(
            output == json!({"done": true}),
            "pending Promise blocked a new call",
        )?;
        if !deadline {
            cancellation.cancel();
        }
        let expected = if deadline {
            ScriptErrorKind::Deadline
        } else {
            ScriptErrorKind::Cancelled
        };
        ensure(
            waiting.await?.is_err_and(|error| error.kind() == expected),
            "pending cancellation/deadline was not delivered",
        )?;
        ensure(
            matches!(runtime.status(), RuntimeStatus::Ready),
            "ordinary wait failure retired the VM",
        )?;
    }
    stop(&runtime).await
}

async fn late_completion(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "let release; let changes = 0; let started = 0; export default { async onRequest(ctx) { started++; ctx.draft = 'uncommitted'; await new Promise(resolve => { release = resolve; }); changes++; ctx.draft = 'late'; }, onFinish() { release(); }, onError(ctx) { ctx.changes = changes; ctx.started = started; } };",
    ).await?;
    for (index, deadline) in [false, true].into_iter().enumerate() {
        let base = u64::try_from(index)? * 10;
        let mut pending = call(
            hook(&config, "onRequest")?,
            base,
            json!({"draft": "original"}),
        );
        if deadline {
            pending.deadline = Instant::now() + Duration::from_millis(500);
        }
        let cancellation = pending.cancellation.clone();
        let pending_runtime = runtime.clone();
        let waiting = tokio::spawn(async move { pending_runtime.invoke(pending).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let entered = runtime
            .invoke(call(hook(&config, "onError")?, base + 3, json!({})))
            .await?;
        ensure(
            entered == json!({"changes": index, "started": index + 1}),
            "late-completion fixture did not enter the pending hook",
        )?;
        if !deadline {
            cancellation.cancel();
        }
        let expected = if deadline {
            ScriptErrorKind::Deadline
        } else {
            ScriptErrorKind::Cancelled
        };
        ensure(
            waiting.await?.is_err_and(|error| error.kind() == expected),
            "failed call exposed a draft instead of an error",
        )?;
        let trigger = runtime
            .invoke(call(hook(&config, "onFinish")?, base + 1, json!({})))
            .await?;
        ensure(trigger == json!({}), "late draft crossed into a new call")?;
        let until = Instant::now() + Duration::from_millis(500);
        loop {
            let observed = runtime
                .invoke(call(hook(&config, "onError")?, base + 2, json!({})))
                .await?;
            if observed == json!({"changes": index + 1, "started": index + 1}) {
                break;
            }
            ensure(
                Instant::now() < until,
                "late continuation did not preserve its closure effect",
            )?;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        ensure(
            matches!(runtime.status(), RuntimeStatus::Ready),
            "late completion retired the VM",
        )?;
    }
    stop(&runtime).await
}

async fn invalid_return(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { const values = [null, {}, [], 1, 'value', true]; if (ctx.kind === 12) return { then(resolve) { resolve(); } }; const value = values[ctx.kind % 6]; return ctx.kind < 6 ? value : Promise.resolve(value); } };",
    ).await?;
    let id = hook(&config, "onRequest")?;
    for kind in 0..=12 {
        let output = runtime.invoke(call(id, kind, json!({"kind": kind}))).await;
        ensure(
            output.is_err_and(|error| error.kind() == ScriptErrorKind::Contract),
            "non-void completion or thenable was accepted",
        )?;
    }
    stop(&runtime).await
}

async fn rejection(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { if (ctx.async) return Promise.reject(new Error('synthetic')); throw new Error('synthetic'); }, onFinish(ctx) { ctx.ok = true; } };",
    ).await?;
    for asynchronous in [false, true] {
        let output = runtime
            .invoke(call(
                hook(&config, "onRequest")?,
                u64::from(asynchronous),
                json!({"async": asynchronous}),
            ))
            .await;
        ensure(
            output.is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
            "throw/rejection was not reported",
        )?;
    }
    let output = runtime
        .invoke(call(hook(&config, "onFinish")?, 3, json!({})))
        .await?;
    ensure(
        output == json!({"ok": true}),
        "ordinary JS failure poisoned the VM",
    )?;
    stop(&runtime).await
}

async fn top_level_await(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "const answer = await Promise.resolve(7); export default { answer, onRequest(ctx) { ctx.answer = answer; ctx.hostApis = [typeof fetch, typeof process, typeof setTimeout]; } };",
    ).await?;
    ensure(
        config["answer"] == 7,
        "top-level await was not awaited before extraction",
    )?;
    let output = runtime
        .invoke(call(hook(&config, "onRequest")?, 1, json!({})))
        .await?;
    ensure(
        output == json!({"answer": 7, "hostApis": ["undefined", "undefined", "undefined"]}),
        "TLA closure or host API boundary failed",
    )?;
    stop(&runtime).await
}

async fn strict_json(fixture: &mut Fixture) -> ProbeResult {
    let invalid = [
        "({value: undefined})",
        "[undefined]",
        "[,]",
        "NaN",
        "Infinity",
        "-Infinity",
        "9007199254740992",
        "-9007199254740992",
        "1n",
        "Symbol('synthetic')",
        "(() => {})",
        "(() => { const value = {}; value.self = value; return value; })()",
    ];
    for expression in invalid {
        let path = fixture.write(
            "config.js",
            &format!("export default {{ data: {expression} }};"),
        )?;
        let result = ScriptRuntime::start(path, RuntimeOptions::default()).await;
        match result {
            Err(error) if error.kind() == ScriptErrorKind::Json => {},
            Err(error) => {
                return Err(format!("invalid static JSON used unexpected error: {error}").into());
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid static JSON was accepted".into());
            },
        }
    }
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { if (ctx.kind === 0) ctx.bad = undefined; if (ctx.kind === 1) ctx.bad = [,]; if (ctx.kind === 2) ctx.bad = 9007199254740992; if (ctx.kind === 3) ctx.bad = ctx; } };",
    ).await?;
    for kind in 0..4 {
        let result = runtime
            .invoke(call(
                hook(&config, "onRequest")?,
                kind,
                json!({"kind": kind}),
            ))
            .await;
        ensure(
            result.is_err_and(|error| error.kind() == ScriptErrorKind::Json),
            "invalid hook JSON was accepted",
        )?;
    }
    let result = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            5,
            json!({"unsafe": 9_007_199_254_740_992_u64}),
        ))
        .await;
    ensure(
        result.is_err_and(|error| error.kind() == ScriptErrorKind::Json),
        "unsafe Rust input integer entered JS",
    )?;
    let result = runtime
        .invoke(call(
            hook(&config, "onRequest")?,
            6,
            json!({"safe": 9_007_199_254_740_991_u64, "null": null}),
        ))
        .await?;
    ensure(
        result == json!({"safe": 9_007_199_254_740_991_u64, "null": null}),
        "safe integer or null failed to round trip",
    )?;
    stop(&runtime).await
}

async fn modules(fixture: &mut Fixture) -> ProbeResult {
    fixture.write("shared/state.mjs", "globalThis.loads = (globalThis.loads ?? 0) + 1; export const token = {}; export const loads = globalThis.loads;")?;
    fixture.write(
        "app/first.js",
        "export { token, loads } from '../shared/state.mjs';",
    )?;
    let path = fixture.write("app/config.js", "import { token as first, loads } from './first.js'; import { token as second } from '../shared/./state.mjs'; export default { same: first === second, loads };")?;
    let (runtime, config) = ScriptRuntime::start(path, RuntimeOptions::default()).await?;
    ensure(
        config == json!({"same": true, "loads": 1}),
        "relative parent imports or normalized module cache failed",
    )?;
    stop(&runtime).await?;
    for specifier in [
        "node:fs",
        "synthetic-package",
        "https://fictional.invalid/config.js",
    ] {
        let path = fixture.write(
            "config.js",
            &format!("import '{specifier}'; export default {{}};"),
        )?;
        match ScriptRuntime::start(path, RuntimeOptions::default()).await {
            Err(error) if error.kind() == ScriptErrorKind::Module => {},
            Err(error) => {
                return Err(
                    format!("forbidden module import used unexpected error: {error}").into(),
                );
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("forbidden module import was accepted".into());
            },
        }
    }
    Ok(())
}

async fn retirement(fixture: &mut Fixture, case: &str) -> ProbeResult {
    let (body, expected) = match case {
        "sync-loop" => ("while (true) {}", ScriptErrorKind::ExecutionBudget),
        "microtask-loop" => (
            "function spin() { Promise.resolve().then(spin); } spin(); return new Promise(() => {});",
            ScriptErrorKind::ExecutionBudget,
        ),
        "memory-limit" => (
            "const retained = []; while (true) retained.push(new Array(8192).fill(1));",
            ScriptErrorKind::MemoryLimit,
        ),
        _ => return Err("invalid retirement experiment".into()),
    };
    let path = fixture.write(
        "config.js",
        &format!("export default {{ onRequest() {{ {body} }} }};"),
    )?;
    let mut options = RuntimeOptions {
        execution_timeout: Duration::from_millis(40),
        continuous_work_timeout: Duration::from_millis(160),
        ..RuntimeOptions::default()
    };
    if case == "memory-limit" {
        options.memory_limit = 8 * 1024 * 1024;
        options.execution_timeout = Duration::from_secs(2);
        options.continuous_work_timeout = Duration::from_secs(3);
    }
    let (runtime, config) = ScriptRuntime::start(path, options).await?;
    let mut changes = runtime.status_changes();
    changes.borrow_and_update();
    let result = runtime
        .invoke(call(hook(&config, "onRequest")?, 1, json!({})))
        .await;
    ensure(
        result.is_err_and(|error| error.kind() == expected),
        "rogue script did not report its resource limit",
    )?;
    // Retiring the VM must wake an HTTP server even when there is no next request.
    while !matches!(*changes.borrow_and_update(), RuntimeStatus::Retired(_)) {
        changes.changed().await?;
    }
    let status = runtime.shutdown().await;
    ensure(
        matches!(status, RuntimeStatus::Retired(error) if error.kind() == expected),
        "resource failure did not retire the VM",
    )?;
    let later = runtime
        .invoke(call(hook(&config, "onRequest")?, 2, json!({})))
        .await;
    ensure(later.is_err(), "retired VM accepted a new call")
}

async fn shutdown_pending(fixture: &mut Fixture) -> ProbeResult {
    let (runtime, config) = fixture.start(
        "export default { onRequest(ctx) { return new Promise(() => { ctx.pending = true; }); }, onFinish(ctx) { ctx.done = true; } };",
    ).await?;
    let pending_runtime = runtime.clone();
    let pending = call(hook(&config, "onRequest")?, 1, json!({}));
    let waiting = tokio::spawn(async move { pending_runtime.invoke(pending).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    stop(&runtime).await?;
    ensure(
        waiting.await?.is_err(),
        "shutdown committed a pending draft",
    )
}

async fn startup_loop(fixture: &mut Fixture) -> ProbeResult {
    for source in [
        "while (true) {} export default {};",
        "await new Promise(() => {}); export default {};",
    ] {
        let path = fixture.write("config.js", source)?;
        let options = RuntimeOptions {
            startup_timeout: Duration::from_millis(200),
            execution_timeout: Duration::from_millis(40),
            ..RuntimeOptions::default()
        };
        match ScriptRuntime::start(path, options).await {
            Err(error)
                if matches!(
                    error.kind(),
                    ScriptErrorKind::ExecutionBudget | ScriptErrorKind::Deadline
                ) => {},
            Err(error) => {
                return Err(format!("startup limit used unexpected error: {error}").into());
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("unbounded startup was accepted".into());
            },
        }
    }
    Ok(())
}
