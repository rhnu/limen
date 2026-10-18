use clap::Parser;

use super::{Cli, Command};

#[test]
fn selects_exact_isolated_names_and_supports_listing() {
    let args = Cli::try_parse_from([
        "xtask",
        "test",
        "--isolated",
        "runtime/sync-loop",
        "http/deadline",
        "--list",
    ])
    .unwrap();
    let Command::Test {
        isolated,
        selection,
    } = args.command;
    assert!(isolated);
    assert_eq!(selection.cases, ["runtime/sync-loop", "http/deadline"]);
    assert!(selection.list);
}

#[test]
fn requires_explicit_isolation_opt_in() {
    for args in [vec!["xtask", "test"], vec!["xtask", "test", "--list"]] {
        assert!(Cli::try_parse_from(args).is_err());
    }
}
