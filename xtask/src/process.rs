//! Process-tree supervision and gateway ownership for isolated experiments.

use std::{
    fs::File,
    io,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};

#[cfg(windows)]
pub mod windows;

pub enum Outcome {
    Exited(ExitStatus),
    TimedOut,
}

/// Retain tree ownership across cancellation until explicit cleanup completes.
struct Tree {
    child: Box<dyn ChildWrapper>,
    armed: bool,
}

impl Tree {
    fn kill(&mut self) -> io::Result<()> {
        match self.child.start_kill() {
            // POSIX ESRCH: the process group already has no live members.
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(3) => Ok(()),
            result => result,
        }
    }

    async fn cleanup(&mut self) -> Result<()> {
        self.kill().context("terminate child process tree")?;
        tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .context("child process tree did not exit after termination")?
            .context("reap child process tree")?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.kill()
        {
            eprintln!("xtask: fallback process-tree termination failed: {error}");
        }
    }
}

/// Bound an entire experiment or Cargo build independently of its executor.
pub async fn supervise(command: tokio::process::Command, timeout: Duration) -> Result<Outcome> {
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessSession);
    #[cfg(windows)]
    command.wrap(process_wrap::tokio::JobObject);
    let mut tree = Tree {
        child: command.spawn().context("spawn supervised child")?,
        armed: true,
    };
    let outcome = tokio::select! {
        status = tree.child.wait() => status.context("wait for supervised child").map(Outcome::Exited),
        () = tokio::time::sleep(timeout) => Ok(Outcome::TimedOut),
        signal = termination() => {
            match signal {
                Ok(()) => Err(anyhow::anyhow!("task interrupted")),
                Err(error) => Err(error).context("observe termination signal"),
            }
        },
    };
    let cleanup = tree.cleanup().await;
    match (outcome, cleanup) {
        (Ok(outcome), Ok(())) => Ok(outcome),
        (Ok(_), Err(error)) | (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("cleanup also failed: {cleanup:#}")))
        },
    }
}

/// Stop tasks on either interactive interruption or a Unix termination signal.
async fn termination() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

/// A worker-owned gateway; the outer supervisor owns its containing process tree.
pub struct Gateway {
    child: Box<dyn process_wrap::std::ChildWrapper>,
}

impl Gateway {
    pub fn spawn(executable: &Path, config: &Path, log: &File) -> Result<Self> {
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new(executable);
            command.arg("serve").arg("--config").arg(config);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg("__gateway").arg(executable).arg(config);
            command
        };
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone().context("clone gateway stdout log")?)
            .stderr(log.try_clone().context("clone gateway stderr log")?);
        let mut command = process_wrap::std::CommandWrap::from(command);
        #[cfg(windows)]
        {
            // The helper allocates and hides its own console before launching limen.
            let mut flags = process_wrap::std::CreationFlags(<_>::default());
            flags.0.0 = 0x0800_0000; // CREATE_NO_WINDOW
            command.wrap(flags);
            command.wrap(process_wrap::std::JobObject);
        }
        Ok(Self {
            child: command.spawn().context("start gateway")?,
        })
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> { self.child.try_wait() }

    /// Poll without blocking the worker's current-thread HTTP executor.
    pub async fn wait_timeout(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().context("poll gateway exit")? {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("gateway did not exit within {timeout:?}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Exercise the application's graceful shutdown rather than killing it.
    pub fn shutdown_signal(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            // SIGTERM is signal 15 on the project's supported Unix platforms.
            self.child.signal(15).context("send SIGTERM to gateway")?;
        }
        #[cfg(windows)]
        windows::signal_helper(self.child.id())?;
        Ok(())
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        match self.child.try_wait() {
            #[cfg(unix)]
            Ok(Some(_)) => return,
            #[cfg(windows)]
            Ok(Some(_)) => {},
            Ok(None) => {},
            Err(error) => eprintln!("xtask: poll gateway during cleanup failed: {error}"),
        }
        if let Err(error) = self.child.start_kill() {
            eprintln!("xtask: terminate gateway during cleanup failed: {error}");
        }
        if let Err(error) = self.child.wait() {
            eprintln!("xtask: reap gateway during cleanup failed: {error}");
        }
    }
}
