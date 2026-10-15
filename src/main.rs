mod bootstrap;
mod cli;
mod gateway;
mod server;
mod telemetry;

use std::process::ExitCode;

use bootstrap::Bootstrap;
use clap::Parser;
use cli::{Cli, Command};

/// The application owns process exit; the VM library only reports retirement.
fn main() -> ExitCode {
    telemetry::install_panic_hook();
    let cli = Cli::parse();
    let (path, serve) = match &cli.command {
        Command::Check(args) => (&args.config, false),
        Command::Serve(args) => (&args.config, true),
    };
    let executor = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(executor) => executor,
        Err(source) => {
            telemetry::report_failure(&bootstrap::Error::Executor(source), "startup", "executor");
            return ExitCode::FAILURE;
        },
    };
    executor.block_on(run(path.as_deref(), serve))
}

async fn run(path: Option<&std::path::Path>, serve: bool) -> ExitCode {
    let command = if serve { "serve" } else { "check" };
    let loaded = match Bootstrap::load(path, command).await {
        Ok(loaded) => loaded,
        Err(error) => {
            telemetry::report_failure(&error, command, error.phase());
            return ExitCode::FAILURE;
        },
    };
    let primary = if serve {
        server::serve(&loaded).await.map_err(bootstrap::Error::from)
    } else {
        Ok(())
    };
    let result = match (primary, loaded.shutdown().await) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(primary), Err(cleanup)) => Err(bootstrap::Error::Cleanup {
            primary: Box::new(primary),
            cleanup: Box::new(cleanup),
        }),
    };
    if let Err(error) = result {
        telemetry::report_failure(&error, command, error.phase());
        return ExitCode::FAILURE;
    }
    if !serve {
        println!(
            "Configuration valid ({} providers, {} models).",
            loaded.business.providers.len(),
            loaded.business.models.len()
        );
    }
    ExitCode::SUCCESS
}
