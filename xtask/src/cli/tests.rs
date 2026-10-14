use clap::Parser;

use super::{Cli, Command, Family};

#[test]
fn selects_named_runtime_experiments() {
    let args = Cli::try_parse_from([
        "xtask",
        "probe",
        "runtime",
        "void",
        "context-stream",
        "--timeout",
        "8",
    ])
    .expect("valid runtime selection");
    let Command::Probe {
        family: Family::Runtime(selection),
    } = args.command
    else {
        panic!("expected runtime experiments");
    };
    assert_eq!(selection.cases, ["void", "context-stream"]);
    assert_eq!(selection.timeout, Some(8.0));
}

#[test]
fn accepts_gateway_paths_with_spaces() {
    let args = Cli::try_parse_from([
        "xtask",
        "probe",
        "http",
        "--executable",
        "build output/limen.exe",
        "--list",
    ])
    .expect("valid HTTP selection");
    let Command::Probe {
        family: Family::Http(selection),
    } = args.command
    else {
        panic!("expected HTTP experiments");
    };
    assert_eq!(
        selection.executable.expect("explicit executable"),
        std::path::PathBuf::from("build output/limen.exe")
    );
    assert!(selection.selection.list);
}

#[test]
fn rejects_unknown_public_commands() {
    assert!(Cli::try_parse_from(["xtask", "serve"]).is_err());
}
