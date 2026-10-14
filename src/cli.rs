use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// The process command line; clap handles help, version and argument errors.
#[derive(Debug, Parser)]
#[command(
    name = "limen",
    version,
    about = "Chat Completions gateway",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Validate configuration, executing local top-level JavaScript.
    #[command(after_help = "Does not invoke request hooks, listen on a port, or call a provider.")]
    Check(ConfigArgs),
    /// Validate configuration and start the HTTP gateway.
    Serve(ConfigArgs),
}

/// Shared configuration options, with native filesystem path parsing.
#[derive(Debug, PartialEq, Eq, Args)]
pub struct ConfigArgs {
    /// Path to the server YAML; config.js is resolved relative to this file.
    #[arg(long, value_name = "PATH", default_value = "server.yaml")]
    pub config: PathBuf,
}

#[cfg(test)]
mod tests;
