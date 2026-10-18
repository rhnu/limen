use std::{ffi::OsString, path::PathBuf};

use clap::error::ErrorKind;

use super::{Cli, Command, ConfigArgs, TestArgs};

#[test]
fn accepts_commands_and_preserves_paths() {
    for (name, expected) in [
        ("check", Command::Check(ConfigArgs { config: None })),
        ("serve", Command::Serve(ConfigArgs { config: None })),
    ] {
        assert_eq!(
            Cli::try_parse_from(["limen", name]).unwrap().command,
            expected
        );
    }
    assert_eq!(
        Cli::try_parse_from(["limen", "test", "--cases", "a directory/用例"])
            .unwrap()
            .command,
        Command::Test(TestArgs {
            configuration: ConfigArgs { config: None },
            cases: PathBuf::from("a directory/用例")
        })
    );
    assert_eq!(
        Cli::try_parse_from(["limen", "serve", "--config", "a directory/服务.yaml"])
            .unwrap()
            .command,
        Command::Serve(ConfigArgs {
            config: Some(PathBuf::from("a directory/服务.yaml"))
        })
    );
    assert_eq!(
        Cli::try_parse_from(["limen", "check", "--config=other.toml"])
            .unwrap()
            .command,
        Command::Check(ConfigArgs {
            config: Some(PathBuf::from("other.toml"))
        })
    );
}

#[cfg(any(unix, windows))]
#[test]
fn preserves_non_utf8_config_paths() {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"config-\xff.yaml".to_vec())
    };
    #[cfg(windows)]
    let path = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[0xD800])
    };
    let cli = Cli::try_parse_from([
        OsString::from("limen"),
        OsString::from("check"),
        OsString::from("--config"),
        path.clone(),
    ])
    .unwrap();
    assert_eq!(
        cli.command,
        Command::Check(ConfigArgs {
            config: Some(PathBuf::from(path))
        })
    );
}

#[test]
fn help_and_version_use_successful_stdout_output() {
    for input in [
        vec!["limen", "--help"],
        vec!["limen", "-h"],
        vec!["limen", "check", "--help"],
        vec!["limen", "serve", "-h"],
        vec!["limen", "test", "--help"],
        vec!["limen", "--version"],
        vec!["limen", "-V"],
    ] {
        let error = Cli::try_parse_from(&input).unwrap_err();
        assert!(matches!(
            error.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        ));
        assert_eq!(error.exit_code(), 0, "{input:?}");
        assert!(!error.use_stderr());
    }
}

#[test]
fn rejects_missing_ambiguous_and_extra_arguments() {
    for input in [
        vec!["limen"],
        vec!["limen", "bad"],
        vec!["limen", "check", "--config"],
        vec!["limen", "check", "--config", "--help"],
        vec!["limen", "check", "--config", ""],
        vec!["limen", "serve", "one.yaml"],
        vec!["limen", "test"],
        vec!["limen", "test", "--cases"],
        vec![
            "limen", "check", "--config", "one.yaml", "--config", "two.yaml",
        ],
        vec!["limen", "--config", "one.yaml", "check"],
    ] {
        let error = Cli::try_parse_from(&input).unwrap_err();
        assert_eq!(error.exit_code(), 2, "{input:?}");
        assert!(error.use_stderr());
    }
}

#[test]
fn argument_errors_do_not_echo_arbitrary_input() {
    let sensitive = "synthetic-private-argument";
    for input in [
        vec!["limen", sensitive],
        vec!["limen", "check", sensitive],
        vec![
            "limen",
            "check",
            "--config",
            sensitive,
            "--config",
            "other.yaml",
        ],
    ] {
        let error = Cli::try_parse_from(input).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(!error.to_string().contains(sensitive));
    }
}
