//! Opt-in VM experiments. Run through tools/runtime-probes.py for a hard process bound.

#[path = "runtime_probe/cases.rs"]
mod cases;

use std::{error::Error, path::PathBuf, process::ExitCode};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("runtime experiment failed: {error}");
            ExitCode::FAILURE
        },
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let case = arguments.next().ok_or("missing experiment name")?;
    let directory = PathBuf::from(
        arguments
            .next()
            .ok_or("missing parent-owned fixture directory")?,
    );
    if arguments.next().is_some() {
        return Err("unexpected argument".into());
    }
    cases::run(case.to_str().ok_or("invalid experiment name")?, directory).await
}
