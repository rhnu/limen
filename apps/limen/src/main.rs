mod bootstrap;
mod cli;
mod telemetry;

use std::process::ExitCode;

use bootstrap::Bootstrap;
use cli::Command;

/// The application owns process exit; the VM library only reports retirement.
fn main() -> ExitCode {
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
            let Ok(executor) = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            else {
                eprintln!("limen: cannot initialize async executor");
                return ExitCode::FAILURE;
            };
            executor.block_on(run(path, serve))
        },
    }
}

async fn run(path: &std::path::Path, serve: bool) -> ExitCode {
    let loaded = match Bootstrap::load(path).await {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("limen: {error}");
            return ExitCode::FAILURE;
        },
    };
    tracing::info!(
        providers = loaded.business.providers.len(),
        models = loaded.business.models.len(),
        "configuration loaded"
    );
    if let Err(error) = loaded.shutdown().await {
        tracing::error!(error = %error, "configuration runtime shutdown failed");
        return ExitCode::FAILURE;
    }
    if serve {
        tracing::error!("HTTP serving is not implemented; configuration validation succeeded");
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
