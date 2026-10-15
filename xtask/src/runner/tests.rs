use std::time::Duration;

use super::{Case, select};
use crate::cli::Selection;

const AVAILABLE: &[Case] = &[
    Case {
        family: "runtime",
        name: "one",
    },
    Case {
        family: "http",
        name: "one",
    },
];

fn selection() -> Selection {
    Selection {
        cases: vec![],
        list: false,
        timeout: None,
        executable: None,
    }
}

#[test]
fn defaults_to_all_families_with_their_own_external_budgets() {
    let selected = select(&selection(), AVAILABLE).unwrap();
    assert_eq!(
        selected,
        [
            (AVAILABLE[0], Duration::from_secs(12)),
            (AVAILABLE[1], Duration::from_secs(18)),
        ]
    );
}

#[test]
fn qualifies_colliding_names_and_preserves_requested_order() {
    let mut args = selection();
    args.cases = vec!["http/one".into(), "runtime/one".into()];
    args.timeout = Some(8.0);
    let selected = select(&args, AVAILABLE).unwrap();
    assert_eq!(
        selected,
        [
            (AVAILABLE[1], Duration::from_secs(8)),
            (AVAILABLE[0], Duration::from_secs(8)),
        ]
    );
}

#[test]
fn rejects_unknown_or_unqualified_cases_before_execution() {
    for name in ["unknown", "one", "runtime/unknown", "other/one"] {
        let mut args = selection();
        args.cases.push(name.into());
        let error = select(&args, AVAILABLE).unwrap_err();
        assert_eq!(
            error.downcast_ref::<clap::Error>().unwrap().kind(),
            clap::error::ErrorKind::InvalidValue
        );
    }
}

#[test]
fn validates_external_deadlines_for_every_selected_family() {
    let mut args = selection();
    for timeout in [f64::NAN, f64::INFINITY, 0.5, 61.0, 4.0] {
        args.timeout = Some(timeout);
        assert!(select(&args, AVAILABLE).is_err());
    }
    args.cases = vec!["runtime/one".into()];
    args.timeout = Some(1.0);
    assert_eq!(
        select(&args, AVAILABLE).unwrap()[0].1,
        Duration::from_secs(1)
    );
    args.cases = vec!["http/one".into()];
    args.timeout = Some(5.0);
    assert_eq!(
        select(&args, AVAILABLE).unwrap()[0].1,
        Duration::from_secs(5)
    );
}
