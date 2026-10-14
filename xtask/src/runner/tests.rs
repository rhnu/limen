use super::select;
use crate::cli::Selection;

#[test]
fn defaults_to_all_experiments_and_preserves_requested_order() {
    let mut args = Selection {
        cases: vec![],
        list: false,
        timeout: None,
    };
    let available = ["one", "two"];
    let (cases, timeout) = select(&args, &available, 1.0, 12.0).expect("default selection");
    assert_eq!(cases, available);
    assert_eq!(timeout.as_secs(), 12);
    args.cases = vec!["two".into(), "one".into()];
    let (cases, _) = select(&args, &available, 1.0, 12.0).expect("named selection");
    assert_eq!(cases, ["two", "one"]);
}

#[test]
fn rejects_unknown_cases_and_invalid_external_deadlines() {
    let mut args = Selection {
        cases: vec!["unknown".into()],
        list: false,
        timeout: None,
    };
    let error = select(&args, &["one"], 1.0, 12.0).expect_err("unknown case");
    assert_eq!(
        error
            .downcast_ref::<clap::Error>()
            .expect("CLI rejection")
            .kind(),
        clap::error::ErrorKind::InvalidValue
    );
    args.cases.clear();
    for timeout in [f64::NAN, f64::INFINITY, 0.5, 61.0] {
        args.timeout = Some(timeout);
        assert!(select(&args, &["one"], 1.0, 12.0).is_err());
    }
    args.timeout = Some(4.0);
    assert!(select(&args, &["one"], 5.0, 18.0).is_err());
}
