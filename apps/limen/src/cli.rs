use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "Limen — Chat Completions gateway configuration\n\nUsage:\n  limen check [--config PATH]\n  limen serve [--config PATH]\n  limen --help\n  limen --version\n\nThe default config path is ./server.yaml. check executes local top-level JavaScript,\nbut does not invoke request hooks, listen on a port, or call a provider.\nserve validates configuration, then reports that HTTP serving is not implemented.";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Check(PathBuf),
    Serve(PathBuf),
    Help,
    Version,
}

/// Parse the small command surface without converting filesystem paths to UTF-8.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, &'static str> {
    let mut args = args.into_iter();
    let first = args.next().ok_or("a command is required; use --help")?;
    if first == "--help" || first == "-h" {
        return no_extra(args, Command::Help);
    }
    if first == "--version" || first == "-V" {
        return no_extra(args, Command::Version);
    }
    let serve = if first == "check" {
        false
    } else if first == "serve" {
        true
    } else {
        return Err("unknown command; use --help");
    };
    let path = match args.next() {
        None => PathBuf::from("server.yaml"),
        Some(flag) if flag == "--help" || flag == "-h" => {
            return no_extra(args, Command::Help);
        },
        Some(flag) if flag == "--config" => {
            let path = args.next().ok_or("--config requires a path")?;
            if path.is_empty() || path.to_string_lossy().starts_with('-') {
                return Err("--config requires a path (prefix dash-leading paths with ./)");
            }
            PathBuf::from(path)
        },
        Some(_) => return Err("unexpected argument; use --help"),
    };
    no_extra(
        args,
        if serve {
            Command::Serve(path)
        } else {
            Command::Check(path)
        },
    )
}

fn no_extra(
    mut args: impl Iterator<Item = OsString>,
    command: Command,
) -> Result<Command, &'static str> {
    if args.next().is_some() {
        Err("unexpected argument; use --help")
    } else {
        Ok(command)
    }
}

#[cfg(test)]
mod tests;
