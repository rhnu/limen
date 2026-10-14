//! Native console operations stay isolated in short-lived helper processes.
#![allow(unsafe_code)]

use std::{
    io,
    os::windows::process::CommandExt,
    path::Path,
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use windows_sys::Win32::{
    System::Console::{
        AllocConsole, AttachConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent,
        GetConsoleWindow, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleCtrlHandler, SetStdHandle,
    },
    UI::WindowsAndMessaging::{SW_HIDE, ShowWindow},
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Launch limen in a private hidden console while preserving its captured logs.
pub fn gateway(executable: &Path, config: &Path) -> Result<ExitCode> {
    let identifiers = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
    // SAFETY: GetStdHandle only queries process-owned handles; they remain open
    // for this helper's lifetime and are not dereferenced or transferred here.
    let handles = identifiers.map(|identifier| unsafe { GetStdHandle(identifier) });
    // SAFETY: This helper has CREATE_NO_WINDOW and no caller console. AllocConsole
    // creates an independent console and takes no pointers or borrowed resources.
    let allocated = unsafe { AllocConsole() };
    ensure!(
        allocated != 0,
        "allocate gateway console: {}",
        io::Error::last_os_error()
    );
    // SAFETY: The window belongs to the console just allocated by this helper.
    let window = unsafe { GetConsoleWindow() };
    if !window.is_null() {
        // SAFETY: The non-null HWND came from GetConsoleWindow; hiding it does not
        // destroy the console required by limen's Ctrl-C handler.
        unsafe { ShowWindow(window, SW_HIDE) };
    }
    for (identifier, handle) in identifiers.into_iter().zip(handles) {
        // SAFETY: Restore the original inherited file handles, still owned by
        // this process, so console allocation cannot redirect captured log output.
        let restored = unsafe { SetStdHandle(identifier, handle) };
        ensure!(
            restored != 0,
            "restore gateway log handle: {}",
            io::Error::last_os_error()
        );
    }
    let mut child = Command::new(executable)
        .arg("serve")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .context("start gateway in private console")?;
    // SAFETY: NULL installs the documented ignore-Ctrl-C behavior only in this
    // helper. Set it AFTER spawning limen, avoiding inheritance of the ignore bit.
    let protected = unsafe { SetConsoleCtrlHandler(None, 1) };
    if protected == 0 {
        let error = io::Error::last_os_error();
        if let Err(cleanup) = child.kill().and_then(|()| child.wait().map(|_| ())) {
            eprintln!("xtask: gateway cleanup also failed: {cleanup}");
        }
        return Err(error).context("protect gateway helper from Ctrl-C");
    }
    let status = child
        .wait()
        .context("wait for gateway in private console")?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Attach only to the gateway helper's private console before broadcasting Ctrl-C.
pub fn signal(pid: u32) -> Result<()> {
    // SAFETY: The signal helper starts with CREATE_NO_WINDOW, so it cannot be
    // attached to the user's terminal. The PID identifies our gateway helper.
    let attached = unsafe { AttachConsole(pid) };
    ensure!(
        attached != 0,
        "attach gateway console: {}",
        io::Error::last_os_error()
    );
    // SAFETY: NULL enables the documented ignore flag only in this signal helper;
    // no child is spawned afterwards and no caller console receives the event.
    let protected = unsafe { SetConsoleCtrlHandler(None, 1) };
    ensure!(
        protected != 0,
        "ignore helper Ctrl-C: {}",
        io::Error::last_os_error()
    );
    // SAFETY: Group 0 broadcasts only in the private console attached above.
    // Windows cannot target CTRL_C_EVENT by nonzero process-group ID.
    let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) };
    ensure!(
        sent != 0,
        "send gateway Ctrl-C: {}",
        io::Error::last_os_error()
    );
    std::thread::sleep(Duration::from_millis(100));
    // SAFETY: Detach this short-lived helper from the console it attached above;
    // FreeConsole does not close the gateway's independent console attachment.
    let detached = unsafe { FreeConsole() };
    ensure!(
        detached != 0,
        "detach gateway console: {}",
        io::Error::last_os_error()
    );
    Ok(())
}

pub fn signal_helper(pid: u32) -> Result<()> {
    let status = Command::new(std::env::current_exe().context("locate signal helper")?)
        .args(["__signal", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("run isolated gateway Ctrl-C helper")?;
    ensure!(status.success(), "gateway Ctrl-C helper failed: {status}");
    Ok(())
}
