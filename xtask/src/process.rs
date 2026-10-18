//! Gateway ownership for process-isolated experiments.

use std::{
    fs::File,
    io,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};

#[cfg(windows)]
pub mod experiments;
#[cfg(windows)]
pub mod windows;

const POLL: Duration = Duration::from_millis(10);
const CLEANUP: Duration = Duration::from_secs(3);

/// Keep native process ownership separate from the private console needed by one signal test.
pub struct Gateway {
    child: GatewayChild,
}

enum GatewayChild {
    Native(Child),
    #[cfg(windows)]
    Signalled(windows::Signalled),
}

impl Gateway {
    pub fn spawn(executable: &Path, config: &Path, log: &File) -> Result<Self> {
        let mut command = Command::new(executable);
        command
            .arg("serve")
            .arg("--config")
            .arg(config)
            .stdin(Stdio::null())
            .stdout(log.try_clone().context("clone gateway stdout log")?)
            .stderr(log.try_clone().context("clone gateway stderr log")?);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        Ok(Self {
            child: GatewayChild::Native(command.spawn().context("start gateway")?),
        })
    }

    /// Use a private pseudoconsole only when exercising the real Windows Ctrl-C path.
    pub fn spawn_signalled(
        executable: &Path,
        config: &Path,
        log: &File,
        log_path: &Path,
    ) -> Result<Self> {
        #[cfg(windows)]
        {
            let _ = log;
            Ok(Self {
                child: GatewayChild::Signalled(windows::Signalled::spawn(
                    executable, config, log_path,
                )?),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = log_path;
            Self::spawn(executable, config, log)
        }
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.child {
            GatewayChild::Native(child) => child.try_wait(),
            #[cfg(windows)]
            GatewayChild::Signalled(child) => child.try_wait(),
        }
    }

    fn start_kill(&mut self) -> io::Result<()> {
        match &mut self.child {
            GatewayChild::Native(child) => {
                if child.try_wait()?.is_none() {
                    child.kill()?;
                }
                Ok(())
            },
            #[cfg(windows)]
            GatewayChild::Signalled(child) => child.start_kill(),
        }
    }

    /// Crash recovery requires the process to exit before reopening SQLite or log files.
    pub fn terminate(&mut self) -> Result<()> {
        self.start_kill().context("force gateway interruption")?;
        let deadline = std::time::Instant::now() + CLEANUP;
        while self
            .try_wait()
            .context("reap interrupted gateway")?
            .is_none()
        {
            if std::time::Instant::now() >= deadline {
                bail!("interrupted gateway did not exit within {CLEANUP:?}");
            }
            std::thread::sleep(POLL);
        }
        #[cfg(windows)]
        if let GatewayChild::Signalled(child) = &mut self.child {
            child.close_blocking(CLEANUP)?;
        }
        Ok(())
    }

    /// Poll without blocking the worker's current-thread HTTP executor.
    pub async fn wait_timeout(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait().context("poll gateway exit")? {
                return Ok(status);
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("gateway did not exit within {timeout:?}");
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Explicit cleanup is awaited before fixture files are read or removed.
    pub async fn close(&mut self) -> Result<()> {
        self.start_kill().context("terminate fixture gateway")?;
        self.wait_timeout(CLEANUP).await?;
        #[cfg(windows)]
        if let GatewayChild::Signalled(child) = &mut self.child {
            child.begin_close();
            let deadline = tokio::time::Instant::now() + CLEANUP;
            while !child.poll_closed()? {
                if tokio::time::Instant::now() >= deadline {
                    bail!("gateway pseudoconsole did not close within {CLEANUP:?}");
                }
                tokio::time::sleep(POLL).await;
            }
        }
        Ok(())
    }

    pub fn shutdown_signal(&mut self) -> Result<()> {
        #[cfg(unix)]
        match &self.child {
            GatewayChild::Native(child) => {
                nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(
                        i32::try_from(child.id()).context("gateway PID exceeds platform range")?,
                    ),
                    nix::sys::signal::Signal::SIGTERM,
                )
                .context("send SIGTERM to gateway")?;
            },
        }
        #[cfg(windows)]
        match &mut self.child {
            GatewayChild::Signalled(child) => child.signal().context("send gateway Ctrl-C")?,
            GatewayChild::Native(_) => {
                bail!("gateway was not started with a private signal console")
            },
        }
        Ok(())
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        if let Err(error) = self.start_kill() {
            eprintln!("isolated: fallback gateway termination failed: {error}");
        }
        // Signalled's Drop moves ClosePseudoConsole off this thread. The process scope
        // owns the final fallback; destructors never wait indefinitely for a child or pipe.
    }
}
