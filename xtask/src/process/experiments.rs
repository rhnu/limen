//! Per-helper jobs release descendant file handles after normal exit and cancelled waits.

use std::{
    fs::{self, OpenOptions},
    os::windows::{fs::OpenOptionsExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use tokio::time::{Instant, sleep, timeout};

const POLL: Duration = Duration::from_millis(10);
const BUDGET: Duration = Duration::from_secs(3);

fn command(case: &str, directory: &Path) -> Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["__helper", "process", case])
        .arg(directory)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

async fn ready(directory: &Path) -> Result<()> {
    timeout(BUDGET, async {
        while !directory.join("ready").exists() {
            sleep(POLL).await;
        }
    })
    .await
    .context("descendants did not acquire their exclusive files")
}

/// Each process owns a nested kill-on-close job before it creates any descendants.
pub async fn helper(case: &str, directory: &Path) -> Result<()> {
    crate::scope::initialize()?;
    match case {
        "root" => {
            let _child = command("locker", directory)?.spawn()?;
            ready(directory).await?;
            timeout(BUDGET, async {
                while !directory.join("release-root").exists() {
                    sleep(POLL).await;
                }
            })
            .await
            .context("root release was never signalled")
        },
        "locker" | "leaf" => {
            let name = if case == "locker" { "parent" } else { "leaf" };
            let _locked = OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(directory.join(format!("{name}.locked")))?;
            fs::write(
                directory.join(format!("{name}.pid")),
                std::process::id().to_string(),
            )?;
            if case == "locker" {
                let _child = command("leaf", directory)?.spawn()?;
                std::future::pending::<()>().await;
            } else {
                fs::write(directory.join("ready"), [])?;
                std::future::pending::<()>().await;
            }
            Ok(())
        },
        _ => bail!("unknown process helper: {case}"),
    }
}

/// Verify OS exit handles and exclusive files, independently of the root child's wait result.
async fn descendants(directory: &Path) -> Result<Vec<super::windows::ProcessExit>> {
    ready(directory).await?;
    let mut observed = Vec::new();
    for name in ["parent", "leaf"] {
        let pid = fs::read_to_string(directory.join(format!("{name}.pid")))?.parse()?;
        let process = super::windows::ProcessExit::open(pid)?;
        ensure!(
            !process.exited()?,
            "descendant exited before the root was released"
        );
        let lock = OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(directory.join(format!("{name}.locked")));
        ensure!(
            matches!(lock, Err(ref error) if error.raw_os_error() == Some(32)),
            "descendant did not retain its exclusive file"
        );
        observed.push(process);
    }
    Ok(observed)
}

async fn released(directory: &Path, observed: &[super::windows::ProcessExit]) -> Result<()> {
    let deadline = Instant::now() + BUDGET;
    for process in observed {
        while !process.exited()? {
            ensure!(
                Instant::now() < deadline,
                "descendant did not exit after its owner's job closed"
            );
            sleep(POLL).await;
        }
    }
    for name in ["parent", "leaf"] {
        let path = directory.join(format!("{name}.locked"));
        let exclusive = OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(&path)
            .context("completed scope left a descendant file locked")?;
        drop(exclusive);
        fs::remove_file(path).context("delete released descendant file")?;
    }
    Ok(())
}

pub async fn run(directory: &Path) -> Result<()> {
    let normal = directory.join("normal");
    fs::create_dir(&normal)?;
    normal_exit(&normal).await?;
    let cancelled = directory.join("cancelled");
    fs::create_dir(&cancelled)?;
    cancelled_wait(&cancelled).await
}

async fn normal_exit(directory: &Path) -> Result<()> {
    let mut child = command("root", directory)?.spawn()?;
    let observed = descendants(directory).await?;
    ensure!(child.try_wait()?.is_none(), "root exited before release");
    fs::write(directory.join("release-root"), [])?;
    let deadline = Instant::now() + BUDGET;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        ensure!(Instant::now() < deadline, "released root did not exit");
        sleep(POLL).await;
    };
    ensure!(status.success(), "normally released root failed");
    ensure!(
        child.try_wait()? == Some(status),
        "repeated root observation changed status"
    );
    released(directory, &observed).await
}

async fn cancelled_wait(directory: &Path) -> Result<()> {
    let mut command = tokio::process::Command::from(command("root", directory)?);
    command.kill_on_drop(true);
    let mut child = command.spawn()?;
    let observed = descendants(directory).await?;
    ensure!(
        timeout(Duration::from_millis(50), child.wait())
            .await
            .is_err(),
        "cancelled wait unexpectedly completed"
    );
    ensure!(
        child.try_wait()?.is_none(),
        "cancelling the wait stopped the root process"
    );
    for process in &observed {
        ensure!(!process.exited()?, "cancelled wait stopped a descendant");
    }
    child.start_kill().context("kill cancelled root")?;
    let status = timeout(BUDGET, child.wait())
        .await
        .context("cancelled root did not exit after termination")??;
    ensure!(!status.success(), "force-terminated root succeeded");
    ensure!(
        child.try_wait()? == Some(status),
        "repeated cancelled root observation changed status"
    );
    released(directory, &observed).await
}
