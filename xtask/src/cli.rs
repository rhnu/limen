//! Explicit development experiments and internal process entry points.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Limen development tasks")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run process-boundary tests in supervised child processes.
    Test {
        /// Explicitly opt in to isolated process tests.
        #[arg(long, required = true)]
        isolated: bool,
        #[command(flatten)]
        selection: Selection,
    },
    #[command(name = "__worker", hide = true)]
    Worker {
        family: String,
        case: String,
        directory: PathBuf,
        executable: Option<PathBuf>,
    },
    #[cfg(windows)]
    #[command(name = "__gateway", hide = true)]
    Gateway {
        executable: PathBuf,
        config: PathBuf,
    },
    #[cfg(windows)]
    #[command(name = "__signal", hide = true)]
    Signal { pid: u32 },
}

#[derive(clap::Args)]
pub struct Selection {
    /// Qualified test names (runtime/name or http/name); omit to run all.
    pub cases: Vec<String>,
    /// List selected tests without building or starting a worker.
    #[arg(long)]
    pub list: bool,
    /// External seconds per test (runtime: 1..60; HTTP: 5..60).
    #[arg(long)]
    pub timeout: Option<f64>,
    /// Reuse an existing gateway instead of building limen for HTTP tests.
    #[arg(long)]
    pub executable: Option<PathBuf>,
}

#[cfg(test)]
mod tests;
