//! Case selection, parent-owned fixtures and experiment results.

use std::{fs, path::Path, process::ExitCode, time::Duration};

use anyhow::{Context, Result};
use clap::error::ErrorKind;

use crate::{cargo, cli, http, process, runtime};

/// Validate a complete selection before building or starting any experiments.
fn select<'a>(
    args: &'a cli::Selection,
    cases: &'a [&str],
    minimum: f64,
    default: f64,
) -> Result<(Vec<&'a str>, Duration)> {
    let seconds = args.timeout.unwrap_or(default);
    if !seconds.is_finite() || !(minimum..=60.0).contains(&seconds) {
        return Err(clap::Error::raw(
            ErrorKind::InvalidValue,
            format!("--timeout must be between {minimum} and 60 seconds"),
        )
        .into());
    }
    for name in &args.cases {
        if !cases.contains(&name.as_str()) {
            return Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                format!("unknown experiment: {name}"),
            )
            .into());
        }
    }
    let selected = if args.cases.is_empty() {
        cases.to_vec()
    } else {
        args.cases.iter().map(String::as_str).collect()
    };
    Ok((selected, Duration::from_secs_f64(seconds)))
}

pub async fn run(family: cli::Family) -> Result<ExitCode> {
    let (name, args, cases, executable, minimum, default) = match family {
        cli::Family::Runtime(args) => ("runtime", args, runtime::CASES, None, 1.0, 12.0),
        cli::Family::Http(args) => (
            "http",
            args.selection,
            http::CASES,
            args.executable,
            5.0,
            18.0,
        ),
    };
    let (selected, timeout) = select(&args, cases, minimum, default)?;
    if args.list {
        for case in cases {
            println!("{case}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let executable = match (name, executable) {
        ("http", Some(path)) => Some(path.canonicalize().context("resolve gateway executable")?),
        ("http", None) => Some(cargo::build_gateway().await?),
        _ => None,
    };
    let mut passed = 0;
    for case in &selected {
        passed += usize::from(run_case(name, case, executable.as_deref(), timeout).await?);
    }
    println!("{passed}/{} {name} experiments passed", selected.len());
    Ok(if passed == selected.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Keep files and process-tree ownership in the process enforcing the deadline.
async fn run_case(
    family: &str,
    case: &str,
    executable: Option<&Path>,
    timeout: Duration,
) -> Result<bool> {
    let started = std::time::Instant::now();
    let directory = tempfile::Builder::new()
        .prefix("limen-probe-")
        .tempdir()
        .context("create experiment directory")?;
    let stdout = directory.path().join("worker.stdout");
    let stderr = directory.path().join("worker.stderr");
    let fixture = directory.path().join("fixture");
    fs::create_dir(&fixture).context("create empty fixture root")?;
    let mut command =
        tokio::process::Command::new(std::env::current_exe().context("locate xtask executable")?);
    command.args(["__worker", family, case]).arg(&fixture);
    if let Some(executable) = executable {
        command.arg(executable);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(fs::File::create(&stdout).context("create worker stdout log")?)
        .stderr(fs::File::create(&stderr).context("create worker stderr log")?);
    let outcome = process::supervise(command, timeout).await;
    let success = matches!(&outcome, Ok(process::Outcome::Exited(status)) if status.success());
    if !success {
        print!(
            "{}",
            fs::read_to_string(&stdout).context("read worker stdout")?
        );
        eprint!(
            "{}",
            fs::read_to_string(&stderr).context("read worker stderr")?
        );
    }
    match &outcome {
        Ok(process::Outcome::TimedOut) => println!(
            "FAIL {case}: killed process tree after {:.2}s",
            timeout.as_secs_f64()
        ),
        Ok(process::Outcome::Exited(status)) if !status.success() => {
            println!("FAIL {case}: worker exit {status}");
        },
        Err(error) => eprintln!("FAIL {case}: {error:#}"),
        _ => {},
    }
    let cleanup = directory.close().context("remove experiment directory");
    match (outcome, cleanup) {
        (Ok(_), Ok(())) => {},
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup)) => {
            return Err(error.context(format!("fixture cleanup also failed: {cleanup:#}")));
        },
    }
    println!(
        "{} {case} ({:.2}s)",
        if success { "PASS" } else { "FAIL" },
        started.elapsed().as_secs_f64()
    );
    Ok(success)
}

#[cfg(test)]
mod tests;
