use std::{ffi::OsString, path::PathBuf};

use super::{Command, parse};

fn args(values: &[&str]) -> Vec<OsString> { values.iter().map(OsString::from).collect() }

#[test]
fn accepts_commands_and_preserves_paths() {
    assert_eq!(
        parse(args(&["check"])).unwrap(),
        Command::Check(PathBuf::from("server.yaml"))
    );
    assert_eq!(
        parse(args(&["serve", "--config", "a directory/服务.yaml"])).unwrap(),
        Command::Serve(PathBuf::from("a directory/服务.yaml"))
    );
    assert_eq!(parse(args(&["check", "--help"])).unwrap(), Command::Help);
    assert_eq!(parse(args(&["--version"])).unwrap(), Command::Version);
}

#[test]
fn rejects_missing_ambiguous_and_extra_arguments() {
    for input in [
        vec![],
        vec!["bad"],
        vec!["check", "--config"],
        vec!["check", "--config", "--help"],
        vec!["check", "--config", ""],
        vec!["--help", "extra"],
        vec!["serve", "one.yaml"],
        vec!["check", "--config", "one.yaml", "--config", "two.yaml"],
    ] {
        assert!(parse(args(&input)).is_err());
    }
}
