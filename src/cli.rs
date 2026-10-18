use std::{ffi::OsString, path::PathBuf};

use clap::{Args, Parser, Subcommand};

/// The process command line; input values never appear in argument error diagnostics.
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

impl Cli {
    pub fn parse() -> Self {
        Self::try_parse_from(std::env::args_os()).unwrap_or_else(|error| error.exit())
    }

    /// Select the safe formatter explicitly: Cargo features are additive, so a
    /// test harness may enable clap's rich error context for the same build.
    pub fn try_parse_from<I, T>(
        args: I,
    ) -> Result<Self, clap::error::Error<clap::error::KindFormatter>>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        <Self as Parser>::try_parse_from(args).map_err(clap::error::Error::apply)
    }
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
