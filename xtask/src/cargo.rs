//! Cargo owns artifact locations; never guess target-directory paths.

use std::{fs, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;

use crate::process::{self, Outcome};

/// Extract the requested application binary, skipping tests and other artifacts.
fn executable(messages: &str, target: &str) -> Result<PathBuf> {
    for line in messages.lines() {
        let message: Value = serde_json::from_str(line).context("decode Cargo message")?;
        if message["reason"] == "compiler-artifact"
            && message["target"]["name"] == target
            && message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
            && message["profile"]["test"] != true
            && let Some(path) = message["executable"].as_str()
        {
            return Ok(PathBuf::from(path));
        }
    }
    bail!("Cargo did not report the {target} executable")
}

pub async fn build_gateway() -> Result<PathBuf> {
    let directory = tempfile::tempdir().context("create Cargo output directory")?;
    let stdout = directory.path().join("stdout");
    let stderr = directory.path().join("stderr");
    let mut command =
        tokio::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .args([
            "build",
            "--locked",
            "--package",
            "limen",
            "--bin",
            "limen",
            "--message-format=json",
        ])
        .current_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .context("xtask must be inside the workspace")?,
        )
        .stdin(std::process::Stdio::null())
        .stdout(fs::File::create(&stdout).context("capture Cargo messages")?)
        .stderr(fs::File::create(&stderr).context("capture Cargo diagnostics")?);
    let outcome = process::supervise(command, Duration::from_secs(240)).await?;
    let messages = fs::read_to_string(&stdout).context("read Cargo messages")?;
    eprint!(
        "{}",
        fs::read_to_string(&stderr).context("read Cargo diagnostics")?
    );
    let Outcome::Exited(status) = outcome else {
        bail!("gateway build exceeded 240 seconds");
    };
    if !status.success() {
        for line in messages.lines() {
            let message: Value = serde_json::from_str(line).context("decode Cargo diagnostic")?;
            if let Some(rendered) = message["message"]["rendered"].as_str() {
                eprint!("{rendered}");
            }
        }
    }
    ensure!(status.success(), "gateway build failed: {status}");
    executable(&messages, "limen")
}

#[cfg(test)]
mod tests;
