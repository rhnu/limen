mod bootstrap;
mod cli;
mod telemetry;

use std::process::ExitCode;

use bootstrap::Bootstrap;
use cli::Command;

/// The application owns process exit; the VM library only reports retirement.
fn main() -> ExitCode {
    telemetry::install_panic_hook();
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("limen: {error}");
            return ExitCode::from(2);
        },
    };
    match command {
        Command::Help => {
            println!("{}", cli::HELP);
            ExitCode::SUCCESS
        },
        Command::Version => {
            println!("limen {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        },
        Command::Check(ref path) | Command::Serve(ref path) => {
            let serve = matches!(&command, Command::Serve(_));
            let executor = match tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            {
                Ok(executor) => executor,
                Err(source) => {
                    telemetry::report_failure(
                        &bootstrap::Error::Executor(source),
                        "startup",
                        "executor",
                    );
                    return ExitCode::FAILURE;
                },
            };
            executor.block_on(run(path, serve))
        },
    }
}

async fn run(path: &std::path::Path, serve: bool) -> ExitCode {
    let command = if serve { "serve" } else { "check" };
    let loaded = match Bootstrap::load(path, command).await {
        Ok(loaded) => loaded,
        Err(error) => {
            telemetry::report_failure(&error, command, error.phase());
            return ExitCode::FAILURE;
        },
    };
    if let Err(error) = loaded.shutdown().await {
        telemetry::report_failure(&error, command, "shutdown");
        return ExitCode::FAILURE;
    }
    if serve {
        tracing::warn!(
            command,
            "HTTP serving is not implemented; configuration validation succeeded"
        );
        ExitCode::from(3)
    } else {
        println!(
            "Configuration valid ({} providers, {} models).",
            loaded.business.providers.len(),
            loaded.business.models.len()
        );
        ExitCode::SUCCESS
    }
}
