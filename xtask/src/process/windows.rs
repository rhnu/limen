//! A private `ConPTY` supplies real Ctrl-C without allocating a visible console window.
#![allow(
    unsafe_code,
    reason = "isolated helpers require Windows process and console APIs"
)]

use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::{Command, ExitCode, ExitStatus, Stdio},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use windows_sys::Win32::{
    Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Console::SetConsoleCtrlHandler,
        Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess,
            WaitForSingleObject,
        },
    },
};

const PID_PREFIX: &[u8] = b"LIMEN_GATEWAY_PID=";
const POLL: Duration = Duration::from_millis(10);
type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// The actual limen process remains observable even if its console helper is terminated first.
pub struct Signalled {
    helper: Helper,
    real: OwnedHandle,
}

struct Helper {
    child: Box<dyn Child + Send + Sync>,
    console: Console,
}

struct Console {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Writer>,
    closed: Option<mpsc::Receiver<()>>,
    drained: mpsc::Receiver<io::Result<()>>,
    close_done: bool,
    drain_done: bool,
}

impl Signalled {
    pub fn spawn(executable: &Path, config: &Path, log_path: &Path) -> Result<Self> {
        let pair = portable_pty::native_pty_system()
            .openpty(PtySize::default())
            .context("create private gateway pseudoconsole")?;
        let reader = pair
            .master
            .try_clone_reader()
            .context("clone gateway console output")?;
        let writer = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .context("take gateway console input")?,
        ));
        let (pid_tx, pid) = mpsc::sync_channel(1);
        let (drain_tx, drained) = mpsc::sync_channel(1);
        let output_writer = Arc::clone(&writer);
        thread::spawn(move || {
            let result = drain_output(reader, &output_writer, pid_tx);
            // Release both pipe handles before publishing completion. A cancelled
            // receiver no longer needs the result and does not require another log.
            drop(output_writer);
            drop(drain_tx.send(result));
        });
        let console = Console {
            master: Some(pair.master),
            writer: Some(writer),
            closed: None,
            drained,
            close_done: false,
            drain_done: false,
        };
        let mut command =
            CommandBuilder::new(std::env::current_exe().context("locate gateway helper")?);
        command.arg("__gateway");
        command.arg(executable);
        command.arg(config);
        command.arg(log_path);
        let child = pair
            .slave
            .spawn_command(command)
            .context("start private-console gateway helper");
        drop(pair.slave);
        let helper = Helper {
            child: child?,
            console,
        };
        // Establish ownership before returning a Gateway. Observing helper exit while
        // its PID message is still queued must never masquerade as descendant cleanup.
        let pid = pid
            .recv_timeout(Duration::from_secs(3))
            .context("receive real gateway PID")?;
        // SAFETY: The PID comes only from the helper's bounded control message.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error()).context("retain real gateway process");
        }
        // SAFETY: A successful OpenProcess transfers this unique handle to OwnedHandle.
        let real = unsafe { OwnedHandle::from_raw_handle(handle) };
        Ok(Self { helper, real })
    }

    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self
            .helper
            .child
            .try_wait()?
            .map(|status| ExitStatus::from_raw(status.exit_code()));
        if !exited(&self.real)? {
            return Ok(None);
        }
        Ok(status)
    }

    pub fn start_kill(&mut self) -> io::Result<()> { terminate(self.real.as_raw_handle()) }

    pub fn signal(&mut self) -> io::Result<()> {
        self.helper
            .console
            .writer
            .as_ref()
            .ok_or_else(|| io::Error::other("gateway console is closed"))?
            .lock()
            .map_err(|_poisoned| io::Error::other("gateway console writer poisoned"))?
            .write_all(&[3])
    }

    pub fn begin_close(&mut self) { self.helper.console.begin_close(); }

    pub fn poll_closed(&mut self) -> Result<bool> { self.helper.console.poll_closed() }

    pub fn close_blocking(&mut self, budget: Duration) -> Result<()> {
        self.begin_close();
        let deadline = Instant::now() + budget;
        while !self.poll_closed()? {
            ensure!(
                Instant::now() < deadline,
                "gateway console cleanup exceeded {budget:?}"
            );
            thread::sleep(POLL);
        }
        Ok(())
    }
}

impl Drop for Signalled {
    fn drop(&mut self) {
        if let Err(error) = self.start_kill() {
            eprintln!("isolated: terminate signalled gateway: {error}");
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(handle) = self.child.as_raw_handle()
            && let Err(error) = terminate(handle)
        {
            eprintln!("isolated: terminate gateway console helper: {error}");
        }
        self.console.begin_close();
    }
}

fn exited(handle: &OwnedHandle) -> io::Result<bool> {
    // SAFETY: OwnedHandle keeps this process handle live throughout the nonblocking query.
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err(io::Error::last_os_error()),
    }
}

fn terminate(handle: std::os::windows::io::RawHandle) -> io::Result<()> {
    // SAFETY: Both call sites hold an owning process object until these calls return.
    if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
        return Ok(());
    }
    // SAFETY: The handle was opened with PROCESS_TERMINATE or comes from our child object.
    if unsafe { TerminateProcess(handle, 1) } == 0 {
        let error = io::Error::last_os_error();
        // SAFETY: The same owner still holds the handle; exit may race with termination.
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            return Err(error);
        }
    }
    Ok(())
}

impl Console {
    fn begin_close(&mut self) {
        if let Some(master) = self.master.take() {
            self.writer.take();
            let (tx, rx) = mpsc::sync_channel(1);
            self.closed = Some(rx);
            // ClosePseudoConsole can emit a final frame. Keep output draining on its own
            // thread and bound only the caller's wait; Drop never joins this worker.
            thread::spawn(move || {
                drop(master);
                // The caller may have stopped waiting after its cleanup deadline.
                let _receiver_closed = tx.send(());
            });
        }
    }

    fn poll_closed(&mut self) -> Result<bool> {
        if !self.close_done
            && let Some(closed) = &self.closed
        {
            match closed.try_recv() {
                Ok(()) => self.close_done = true,
                Err(mpsc::TryRecvError::Empty) => {},
                Err(mpsc::TryRecvError::Disconnected) => {
                    bail!("gateway console closer stopped without completion")
                },
            }
        }
        if !self.drain_done {
            match self.drained.try_recv() {
                Ok(result) => {
                    result.context("drain gateway console output")?;
                    self.drain_done = true;
                },
                Err(mpsc::TryRecvError::Empty) => {},
                Err(mpsc::TryRecvError::Disconnected) => {
                    bail!("gateway console reader stopped without completion")
                },
            }
        }
        Ok(self.close_done && self.drain_done)
    }
}

impl Drop for Console {
    fn drop(&mut self) { self.begin_close(); }
}

/// Discard terminal presentation bytes using fixed buffers; diagnostics use the original file.
fn drain_output(
    mut reader: Box<dyn Read + Send>,
    writer: &Writer,
    pid: mpsc::SyncSender<u32>,
) -> io::Result<()> {
    let mut buffer = [0u8; 4096];
    let mut line = Vec::with_capacity(128);
    let mut cursor = 0;
    let mut pid = Some(pid);
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        for byte in &buffer[..count] {
            // portable-pty 0.9 enables cursor inheritance. Reply even when the query
            // is split across reads; leaving it unanswered can deadlock shutdown.
            let query = b"\x1b[6n";
            if *byte == query[cursor] {
                cursor += 1;
            } else {
                cursor = usize::from(*byte == query[0]);
            }
            if cursor == query.len() {
                writer
                    .lock()
                    .map_err(|_poisoned| io::Error::other("gateway console writer poisoned"))?
                    .write_all(b"\x1b[1;1R")?;
                cursor = 0;
            }
            if *byte == b'\n' {
                if let Some(sender) = pid.take() {
                    if let Some(start) = line
                        .windows(PID_PREFIX.len())
                        .position(|part| part == PID_PREFIX)
                    {
                        let digits = line[start + PID_PREFIX.len()..]
                            .iter()
                            .copied()
                            .take_while(u8::is_ascii_digit)
                            .collect::<Vec<_>>();
                        let value = std::str::from_utf8(&digits)
                            .ok()
                            .and_then(|s| s.parse().ok());
                        if let Some(value) = value {
                            sender.send(value).map_err(|_ignored| {
                                io::Error::other("gateway PID receiver closed")
                            })?;
                        } else {
                            return Err(io::Error::other("gateway helper emitted an invalid PID"));
                        }
                    } else {
                        pid = Some(sender);
                    }
                }
                line.clear();
            } else if pid.is_some() {
                if line.len() == 128 {
                    line.remove(0);
                }
                line.push(*byte);
            }
        }
    }
}

unsafe extern "system" fn protect_helper(event: u32) -> i32 { i32::from(event == 0) }

/// Start limen with original JSON output handles while inheriting only the private console.
pub fn gateway(executable: &Path, config: &Path, log_path: &Path) -> Result<ExitCode> {
    crate::scope::initialize()?;
    // SAFETY: The static callback has no captures, touches no shared state, and only
    // protects this helper. Custom handlers are not inherited by the child process.
    let protected = unsafe { SetConsoleCtrlHandler(Some(protect_helper), 1) };
    ensure!(
        protected != 0,
        "protect gateway helper from Ctrl-C: {}",
        io::Error::last_os_error()
    );
    let log = OpenOptions::new()
        .append(true)
        .open(log_path)
        .context("open raw gateway diagnostic log")?;
    let mut child = Command::new(executable)
        .arg("serve")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("start gateway in private console")?;
    println!(
        "{}{}",
        std::str::from_utf8(PID_PREFIX).expect("control prefix is ASCII"),
        child.id()
    );
    io::stdout().flush().context("publish real gateway PID")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().context("poll private-console gateway")? {
            return Ok(if status.success() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        ensure!(
            Instant::now() < deadline,
            "private-console gateway exceeded helper lifetime"
        );
        thread::sleep(POLL);
    }
}

/// Retain a descendant's OS process object across root exit and job termination.
pub struct ProcessExit(OwnedHandle);

impl ProcessExit {
    pub fn open(pid: u32) -> io::Result<Self> {
        // SAFETY: This only opens an observation handle for the helper's published PID.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcess returned a unique owned handle transferred exactly once.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    pub fn exited(&self) -> io::Result<bool> {
        // SAFETY: OwnedHandle remains live for this nonblocking wait.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            windows_sys::Win32::Foundation::WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
}
