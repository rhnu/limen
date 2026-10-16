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
    #[command(
        after_help = "Does not create runtime data/logs, invoke request hooks, listen, or call a provider."
    )]
    Check(ConfigArgs),
    /// Validate configuration and start the HTTP gateway.
    Serve(ConfigArgs),
    /// Test local scripts against synthetic upstream responses without networking.
    Test(TestArgs),
}

#[derive(Debug, PartialEq, Eq, Args)]
pub struct TestArgs {
    #[command(flatten)]
    pub configuration: ConfigArgs,
    /// One .case.json file or a directory containing .case.json files.
    #[arg(long, value_name = "PATH")]
    pub cases: PathBuf,
}

/// Shared configuration options, with native filesystem path parsing.
#[derive(Debug, PartialEq, Eq, Args)]
pub struct ConfigArgs {
    /// Server TOML/YAML path; defaults only to ./config/server.toml.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

#[cfg(test)]
mod tests;
