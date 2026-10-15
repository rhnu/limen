use clap::Parser;

use super::{Cli, Command};

#[test]
fn selects_qualified_isolated_tests_in_requested_order() {
    let args = Cli::try_parse_from([
        "xtask",
        "test",
        "--isolated",
        "runtime/sync-loop",
        "http/deadline",
        "--timeout",
        "8",
    ])
    .expect("valid isolated test selection");
    let Command::Test {
        isolated,
        selection,
    } = args.command
    else {
        panic!("expected isolated tests");
    };
    assert!(isolated);
    assert_eq!(selection.cases, ["runtime/sync-loop", "http/deadline"]);
    assert_eq!(selection.timeout, Some(8.0));
}

#[test]
fn accepts_gateway_paths_with_spaces_and_lists_without_running() {
    let args = Cli::try_parse_from([
        "xtask",
        "test",
        "--isolated",
        "--executable",
        "build output/limen.exe",
        "--list",
    ])
    .expect("valid isolated listing");
    let Command::Test { selection, .. } = args.command else {
        panic!("expected isolated tests");
    };
    assert_eq!(
        selection.executable.unwrap(),
        std::path::PathBuf::from("build output/limen.exe")
    );
    assert!(selection.list);
}

#[test]
fn requires_explicit_isolation_opt_in() {
    for args in [
        vec!["xtask", "test"],
        vec!["xtask", "test", "--list"],
        vec!["xtask", "test", "--timeout", "8"],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}
