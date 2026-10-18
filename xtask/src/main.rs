//! Development orchestration; nextest owns isolated test execution.

mod cli;
mod runner;
mod scope;

use std::process::ExitCode;

use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args = cli::Cli::parse();
    let result = async {
        #[cfg(windows)]
        scope::initialize()?;
        let cli::Command::Test { selection, .. } = args.command;
        runner::run(selection).await
    }
    .await;
    match result {
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
