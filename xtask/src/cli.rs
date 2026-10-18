//! Explicit opt-in to the nextest isolation profile.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Limen development tasks")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run boundary tests with nextest's per-test process isolation.
    Test {
        #[arg(long, required = true)]
        isolated: bool,
        #[command(flatten)]
        selection: Selection,
    },
}

#[derive(clap::Args)]
pub struct Selection {
    /// Exact qualified names from --list; omit to run all isolated tests.
    pub cases: Vec<String>,
    /// Build and list available tests without executing them.
    #[arg(long)]
    pub list: bool,
}

#[cfg(test)]
mod tests;
