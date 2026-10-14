//! Development automation; experiments always execute in supervised children.

mod cargo;
mod cli;
mod http;
mod process;
mod runner;
mod runtime;

use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(cli::Cli::parse()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            if error.is::<clap::Error>() {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        },
    }
}

/// Dispatch public tasks separately from process-isolated worker entry points.
async fn run(args: cli::Cli) -> Result<ExitCode> {
    match args.command {
        cli::Command::Probe { family } => runner::run(family).await,
        cli::Command::Worker {
            family,
            case,
            directory,
            executable,
        } => {
            match family.as_str() {
                "runtime" => runtime::run(&case, directory)
                    .await
                    .map_err(anyhow::Error::from_boxed)?,
                "http" => {
                    let executable = executable.context("HTTP worker requires a gateway")?;
                    http::run(&case, &executable, &directory).await?;
                },
                _ => bail!("unknown experiment family: {family}"),
            }
            Ok(ExitCode::SUCCESS)
        },
        #[cfg(windows)]
        cli::Command::Gateway { executable, config } => {
            process::windows::gateway(&executable, &config)
        },
        #[cfg(windows)]
        cli::Command::Signal { pid } => {
            process::windows::signal(pid)?;
            Ok(ExitCode::SUCCESS)
        },
    }
}
