//! Build and select nextest cases while retaining ownership of their temporary root.

use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::Path,
    process::{ExitCode, ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use clap::error::ErrorKind;
use serde_json::Value;
use tokio::process::Command;

use crate::{cli, scope};

fn command(list: bool) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.args([
        "nextest",
        if list { "list" } else { "run" },
        "--locked",
        "--package",
        "limen",
        "--test",
        "isolated",
        "--features",
        "isolated-tests",
        "--profile",
        "isolated",
        "--color",
        "never",
    ]);
    command.current_dir(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask belongs to the workspace"),
    );
    command.stdin(Stdio::null()).kill_on_drop(true);
    scope::configure(&mut command);
    command
}

/// Preserve exact-name semantics, including rejecting a missing member of a mixed selection.
fn select(requested: &[String], available: &BTreeSet<String>) -> Result<BTreeSet<String>> {
    if requested.is_empty() {
        return Ok(available.clone());
    }
    requested
        .iter()
        .map(|name| {
            if !available.contains(name) {
                return Err(clap::Error::raw(
                    ErrorKind::InvalidValue,
                    format!("unknown isolated test: {name}"),
                )
                .into());
            }
            Ok(name.clone())
        })
        .collect()
}

fn catalogue(value: &Value) -> Result<BTreeSet<String>> {
    let suites = value["rust-suites"]
        .as_object()
        .context("nextest did not report test suites")?;
    let suite = suites
        .get("limen::isolated")
        .context("nextest did not report the isolated test target")?;
    let cases = suite["testcases"]
        .as_object()
        .context("nextest did not report isolated cases")?;
    ensure!(!cases.is_empty(), "isolated test target contains no cases");
    Ok(cases.keys().cloned().collect())
}

/// Forward Unix signals; Windows children inherit our console and receive its event directly.
async fn interruption() -> Result<i32> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            signal = tokio::signal::ctrl_c() => { signal?; Ok(2) },
            _ = terminate.recv() => Ok(15),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
        Ok(2)
    }
}

async fn finish(mut command: Command, budget: Duration) -> Result<ExitStatus> {
    let mut child = command
        .spawn()
        .context("start nextest (install cargo-nextest 0.9.148 or newer)")?;
    let failure = tokio::select! {
        status = child.wait() => return status.context("wait for nextest"),
        () = tokio::time::sleep(budget) => anyhow::anyhow!("nextest exceeded its outer {budget:?} budget"),
        signal = interruption() => {
            let signal = signal?;
            #[cfg(unix)]
            if let Some(id) = child.id() {
                use nix::{sys::signal::{Signal, kill}, unistd::Pid};
                let pid = i32::try_from(id).context("nextest PID exceeds platform range")?;
                let signal = if signal == 15 { Signal::SIGTERM } else { Signal::SIGINT };
                match kill(Pid::from_raw(pid), signal) {
                    Ok(()) | Err(nix::errno::Errno::ESRCH) => {},
                    Err(error) => return Err(error).context("forward interruption to nextest"),
                }
            }
            #[cfg(windows)]
            let _ = signal;
            match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
                Ok(Ok(_)) => bail!("isolated test run interrupted"),
                Ok(Err(error)) => return Err(error).context("wait for interrupted nextest"),
                Err(_) => anyhow::anyhow!("nextest did not stop after interruption"),
            }
        },
    };
    child
        .start_kill()
        .context("terminate nextest after its outer budget")?;
    let cleanup = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    match cleanup {
        Ok(Ok(_)) => Err(failure),
        Ok(Err(error)) => Err(failure.context(format!("nextest cleanup also failed: {error}"))),
        Err(error) => Err(failure.context(format!("nextest cleanup also timed out: {error}"))),
    }
}

pub async fn run(args: cli::Selection) -> Result<ExitCode> {
    let directory = tempfile::Builder::new()
        .prefix("limen-isolated-")
        .tempdir()
        .context("create isolated run root")?;
    let root = directory.path().to_owned();
    let result = async {
        let output = root.join("catalogue.json");
        let mut listing = command(true);
        listing
            .args(["--message-format", "json"])
            .stdout(File::create(&output)?);
        ensure!(
            finish(listing, Duration::from_secs(240)).await?.success(),
            "nextest could not build or list isolated tests"
        );
        let mut bytes = Vec::new();
        File::open(output)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= 1024 * 1024,
            "nextest catalogue exceeded 1 MiB"
        );
        let available =
            catalogue(&serde_json::from_slice(&bytes).context("decode nextest catalogue")?)?;
        let selected = select(&args.cases, &available)?;
        if args.list {
            for name in selected {
                println!("{name}");
            }
            return Ok(ExitCode::SUCCESS);
        }
        let mut execution = command(false);
        execution
            .env("LIMEN_ISOLATED_ROOT", &root)
            .args(["--no-tests", "fail"]);
        if selected != available {
            let filter = selected
                .iter()
                .map(|name| format!("test(={name})"))
                .collect::<Vec<_>>()
                .join(" | ");
            execution.args(["-E", &filter]);
        }
        let status = finish(execution, Duration::from_mins(16)).await?;
        Ok(if status.success() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        })
    }
    .await;
    let cleanup = directory.close().with_context(|| {
        format!(
            "remove isolated run root; retained on failure: {}",
            root.display()
        )
    });
    match (result, cleanup) {
        (Ok(code), Ok(())) => Ok(code),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        },
    }
}

#[cfg(test)]
mod tests;
