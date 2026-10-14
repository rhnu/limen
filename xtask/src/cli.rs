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
    /// Run isolated experiments outside ordinary unit tests.
    Probe {
        #[command(subcommand)]
        family: Family,
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

#[derive(Subcommand)]
pub enum Family {
    /// JS and VM contracts, each in a separate process.
    Runtime(Selection),
    /// Local synthetic HTTP upstream and gateway experiments.
    Http(HttpSelection),
}

#[derive(clap::Args)]
pub struct Selection {
    /// Experiment names; omit to run the complete family.
    pub cases: Vec<String>,
    /// List experiments without starting a worker.
    #[arg(long)]
    pub list: bool,
    /// External seconds per experiment (runtime: 1..60; HTTP: 5..60).
    #[arg(long)]
    pub timeout: Option<f64>,
}

#[derive(clap::Args)]
pub struct HttpSelection {
    #[command(flatten)]
    pub selection: Selection,
    /// Reuse an existing gateway instead of building limen.
    #[arg(long)]
    pub executable: Option<PathBuf>,
}

#[cfg(test)]
mod tests;
