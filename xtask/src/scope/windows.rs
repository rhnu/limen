//! Console presence uses native code; Job ownership uses a safe API.
#![allow(unsafe_code, reason = "query the calling process's Windows console")]

use std::sync::OnceLock;

use anyhow::{Context, Result};
use win32job::{ExtendedLimitInfo, Job};

// Statics live until process teardown. Closing the final Job handle while the
// owner is still running would terminate the owner as well as its descendants.
static OWNER: OnceLock<Job> = OnceLock::new();

pub(super) fn initialize() -> Result<()> {
    if OWNER.get().is_some() {
        return Ok(());
    }
    let job = Job::create_with_limit_info(ExtendedLimitInfo::new().limit_kill_on_job_close())
        .context("create process-lifetime Windows Job")?;
    job.assign_current_process()
        .context("enroll process before it can spawn descendants")?;
    OWNER
        .set(job)
        .map_err(|_uninstalled_owner| anyhow::anyhow!("process owner initialized twice"))?;
    Ok(())
}

pub(super) fn creation_flags() -> u32 {
    // SAFETY: GetConsoleWindow takes no pointers and only queries this process.
    if unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null() {
        0x0800_0000 // CREATE_NO_WINDOW
    } else {
        0
    }
}
