//! Process-lifetime ownership prevents descendants outliving an isolated owner.

#[cfg(windows)]
use anyhow::Result;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub fn initialize() -> Result<()> {
    windows::initialize()?;
    Ok(())
}

/// Inherit an existing console, or explicitly suppress creating a new one.
pub fn configure(command: &mut tokio::process::Command) {
    #[cfg(windows)]
    command.creation_flags(windows::creation_flags());
    #[cfg(not(windows))]
    let _ = command;
}
