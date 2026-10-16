//! Isolated case selection, parent-owned fixtures and supervised test results.

use std::{fs, path::Path, process::ExitCode, time::Duration};

use anyhow::{Context, Result};
use clap::error::ErrorKind;

use crate::{cargo, cli, http, process, runtime, storage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Case {
    family: &'static str,
    name: &'static str,
}

impl Case {
    fn qualified(self) -> String { format!("{}/{}", self.family, self.name) }

    fn timeout(self, seconds: Option<f64>) -> Result<Duration> {
        let (minimum, default) = if self.family == "runtime" {
            (1.0, 12.0)
        } else {
            (5.0, 18.0)
        };
        let seconds = seconds.unwrap_or(default);
        if !seconds.is_finite() || !(minimum..=60.0).contains(&seconds) {
            return Err(clap::Error::raw(
                ErrorKind::InvalidValue,
                format!(
                    "--timeout for {} must be between {minimum} and 60 seconds",
                    self.qualified()
                ),
            )
            .into());
        }
        Ok(Duration::from_secs_f64(seconds))
    }
}

fn cases() -> Vec<Case> {
    [
        ("runtime", runtime::CASES),
        ("http", http::CASES),
        ("storage", storage::CASES),
    ]
    .into_iter()
    .flat_map(|(family, names)| names.iter().map(move |name| Case { family, name }))
    .collect()
}

/// Validate the complete selection and all budgets before building or starting a worker.
fn select(args: &cli::Selection, available: &[Case]) -> Result<Vec<(Case, Duration)>> {
    let selected = if args.cases.is_empty() {
        available.to_vec()
    } else {
        args.cases
            .iter()
            .map(|name| {
                available
                    .iter()
                    .copied()
                    .find(|case| case.qualified() == *name)
                    .ok_or_else(|| {
                        clap::Error::raw(
                            ErrorKind::InvalidValue,
                            format!("unknown isolated test: {name}"),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    selected
        .into_iter()
        .map(|case| Ok((case, case.timeout(args.timeout)?)))
        .collect()
}

pub async fn run(args: cli::Selection) -> Result<ExitCode> {
    let selected = select(&args, &cases())?;
    if args.list {
        for (case, _) in selected {
            println!("{}", case.qualified());
        }
        return Ok(ExitCode::SUCCESS);
    }
    let needs_gateway = selected.iter().any(|(case, _)| case.family == "http");
    let executable = if needs_gateway {
        Some(match args.executable {
            Some(path) => path.canonicalize().context("resolve gateway executable")?,
            None => cargo::build_gateway().await?,
        })
    } else {
        None
    };
    let mut passed = 0;
    for (case, timeout) in &selected {
        passed += usize::from(run_case(*case, executable.as_deref(), *timeout).await?);
    }
    println!("{passed}/{} isolated tests passed", selected.len());
    Ok(if passed == selected.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Keep files and process-tree ownership in the process enforcing the deadline.
async fn run_case(test: Case, executable: Option<&Path>, timeout: Duration) -> Result<bool> {
    let Case { family, name: case } = test;
    let qualified = test.qualified();
    let started = std::time::Instant::now();
    let directory = tempfile::Builder::new()
        .prefix("limen-test-")
        .tempdir()
        .context("create isolated test directory")?;
    let stdout = directory.path().join("worker.stdout");
    let stderr = directory.path().join("worker.stderr");
    let fixture = directory.path().join("fixture");
    fs::create_dir(&fixture).context("create empty fixture root")?;
    let mut command =
        tokio::process::Command::new(std::env::current_exe().context("locate xtask executable")?);
    command.args(["__worker", family, case]).arg(&fixture);
    if family == "runtime" && case == "environment" {
        runtime::environment::configure(&mut command);
    }
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
            "FAIL {qualified}: killed process tree after {:.2}s",
            timeout.as_secs_f64()
        ),
        Ok(process::Outcome::Exited(status)) if !status.success() => {
            println!("FAIL {qualified}: worker exit {status}");
        },
        Err(error) => eprintln!("FAIL {qualified}: {error:#}"),
        _ => {},
    }
    let cleanup = directory.close().context("remove isolated test directory");
    match (outcome, cleanup) {
        (Ok(_), Ok(())) => {},
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup)) => {
            return Err(error.context(format!("fixture cleanup also failed: {cleanup:#}")));
        },
    }
    println!(
        "{} {qualified} ({:.2}s)",
        if success { "PASS" } else { "FAIL" },
        started.elapsed().as_secs_f64()
    );
    Ok(success)
}

#[cfg(test)]
mod tests;
