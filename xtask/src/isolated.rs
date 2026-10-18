//! A nextest-only harness; production behavior stays in the application and its libraries.

mod http;
mod process;
mod runtime;
mod scope;
mod storage;

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use libtest_mimic::{Arguments, Failed, Trial};

#[derive(Clone, Copy)]
struct Case {
    family: &'static str,
    name: &'static str,
}

impl Case {
    fn qualified(self) -> String { format!("{}/{}", self.family, self.name) }
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

fn executor<T>(action: impl Future<Output = T>) -> Result<T> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create isolated executor")?;
    let result = executor.block_on(action);
    // The external process deadline, rather than dropping a stuck VM thread,
    // owns the final bound on an experiment that has stopped making progress.
    executor.shutdown_timeout(Duration::ZERO);
    Ok(result)
}

async fn environment(directory: &Path) -> Result<()> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .arg("__helper")
        .arg("environment")
        .arg(directory)
        .kill_on_drop(true);
    runtime::environment::configure(&mut command);
    scope::configure(&mut command);
    ensure!(
        command
            .status()
            .await
            .context("wait for inherited-environment probe")?
            .success(),
        "inherited-environment probe failed"
    );
    Ok(())
}

fn run_case(case: Case) -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("LIMEN_ISOLATED_ROOT")
            .context("run isolated tests through cargo xtask test --isolated")?,
    );
    let directory = root.join(format!("{}-{}", case.family, case.name));
    fs::create_dir(&directory).context("create isolated fixture directory")?;
    executor(async {
        match case.family {
            "runtime" if case.name == "environment" => environment(&directory).await,
            "runtime" => runtime::run(case.name, directory)
                .await
                .map_err(anyhow::Error::from_boxed),
            "http" => {
                http::run(
                    case.name,
                    Path::new(env!("CARGO_BIN_EXE_limen")),
                    &directory,
                )
                .await
            },
            "storage" => storage::run(case.name, &directory).await,
            _ => bail!("unknown isolated test family"),
        }
    })?
}

fn helper(args: &[OsString]) -> Result<ExitCode> {
    #[cfg(windows)]
    scope::initialize()?;
    match args.first().and_then(|arg| arg.to_str()) {
        Some("__helper") if args.get(1).is_some_and(|arg| arg == "environment") => {
            ensure!(args.len() == 3, "environment helper requires a directory");
            executor(runtime::run("environment", PathBuf::from(&args[2])))?
                .map_err(anyhow::Error::from_boxed)?;
        },
        #[cfg(windows)]
        Some("__helper") if args.get(1).is_some_and(|arg| arg == "process") => {
            ensure!(
                args.len() == 4,
                "process helper requires a case and directory"
            );
            let case = args[2]
                .to_str()
                .context("process helper name is not UTF-8")?;
            executor(process::experiments::helper(case, Path::new(&args[3])))??;
        },
        #[cfg(windows)]
        Some("__gateway") => {
            ensure!(
                args.len() == 4,
                "gateway helper requires executable, config and log paths"
            );
            return process::windows::gateway(
                Path::new(&args[1]),
                Path::new(&args[2]),
                Path::new(&args[3]),
            );
        },
        _ => bail!("unknown isolated helper"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let raw = std::env::args_os().skip(1).collect::<Vec<_>>();
    if raw
        .first()
        .is_some_and(|arg| arg == "__helper" || arg == "__gateway")
    {
        return match helper(&raw) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("isolated helper: {error:#}");
                ExitCode::FAILURE
            },
        };
    }
    let args = Arguments::from_args();
    if !args.list {
        if std::env::var("NEXTEST_EXECUTION_MODE").as_deref() != Ok("process-per-test") {
            eprintln!(
                "isolated tests require cargo xtask test --isolated; running them through cargo test is unsupported"
            );
            return ExitCode::FAILURE;
        }
        #[cfg(windows)]
        if let Err(error) = scope::initialize() {
            eprintln!("isolated scope: {error:#}");
            return ExitCode::FAILURE;
        }
    }
    let trials = cases()
        .into_iter()
        .map(|case| {
            Trial::test(case.qualified(), move || {
                run_case(case).map_err(|error| Failed::from(format!("{error:#}")))
            })
        })
        .collect();
    libtest_mimic::run(&args, trials).exit();
}
