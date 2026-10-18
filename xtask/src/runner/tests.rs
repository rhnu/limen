use std::collections::BTreeSet;

use super::{catalogue, select};

#[test]
fn exact_selection_rejects_unknown_members_in_a_mixed_request() {
    let available = BTreeSet::from(["runtime/one".into(), "http/one".into()]);
    assert_eq!(select(&[], &available).unwrap(), available);
    assert_eq!(
        select(&["http/one".into()], &available).unwrap(),
        BTreeSet::from(["http/one".into()])
    );
    let error = select(&["http/one".into(), "runtime/missing".into()], &available).unwrap_err();
    assert_eq!(
        error.downcast_ref::<clap::Error>().unwrap().kind(),
        clap::error::ErrorKind::InvalidValue
    );
}

#[test]
fn reads_the_isolated_catalogue_and_rejects_missing_targets() {
    let input =
        serde_json::json!({"rust-suites": {"limen::isolated": {"testcases": {"runtime/one": {}}}}});
    assert_eq!(
        catalogue(&input).unwrap(),
        BTreeSet::from(["runtime/one".into()])
    );
    assert!(catalogue(&serde_json::json!({"rust-suites": {}})).is_err());
    assert!(
        catalogue(&serde_json::json!({"rust-suites": {"limen::isolated": {"testcases": {}}}}))
            .is_err()
    );
}
