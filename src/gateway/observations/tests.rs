use std::{
    io,
    sync::{Arc, Mutex},
};

use super::*;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer { self.clone() }
}

#[test]
fn exported_statistics_and_observations_do_not_include_arbitrary_upstream_fields() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_writer(capture.clone())
        .finish();
    let mut statistics = super::super::stats::Statistics::default();
    statistics.usage.observe(Some(
        &json!({"prompt_tokens":2,"completion_tokens":3,"total_tokens":5,
        "private":"synthetic-body-token-cookie-query-secret"}),
    ));
    let mut observations = Observations::new(2);
    observations.commit(
        ObservationBatch {
            events: vec![0],
            dropped: 0,
        },
        "beforeSend",
        Some("policy"),
    );
    crate::telemetry::tests::with_test_subscriber(subscriber, || {
        super::super::stats::record("request-1", &statistics.snapshot());
        observations.record("request-1", &["policy_applied".into()]);
    });
    let text = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(!text.contains("synthetic-body-token-cookie-query-secret"));
    let events: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["fields"].clone())
        .collect();
    assert!(events[0].get("input_tokens").is_none());
    assert_eq!(events[0]["message"], "request timing");
    assert_eq!(events[1]["event"], "policy_applied");
    assert_eq!(events[1]["phase"], "beforeSend");
    assert_eq!(events[1]["count"], 1);
}

#[test]
fn aggregation_is_scoped_ordered_and_request_capacity_counts_each_committed_occurrence() {
    let names = vec!["first".into(), "second".into()];
    let mut observations = Observations::new(3);
    observations.commit(
        ObservationBatch {
            events: vec![0],
            dropped: 1,
        },
        "beforeSend",
        Some("policy"),
    );
    observations.commit(
        ObservationBatch {
            events: vec![0],
            dropped: 0,
        },
        "beforeSend",
        Some("policy"),
    );
    observations.commit(
        ObservationBatch {
            events: vec![1, 0],
            dropped: 0,
        },
        "onFinish",
        None,
    );
    assert_eq!(
        observations.snapshot(&names),
        json!({"events":[
        {"event":"first","phase":"beforeSend","step":"policy","count":2},
        {"event":"second","phase":"onFinish","step":null,"count":1}
    ],"dropped":2})
    );
}

#[test]
fn empty_and_disabled_capacity_have_no_unbounded_storage() {
    let mut observations = Observations::default();
    observations.commit(
        ObservationBatch {
            events: vec![0],
            dropped: u64::MAX,
        },
        "onRequest",
        None,
    );
    let snapshot = observations.snapshot(&["only".into()]);
    assert_eq!(snapshot["events"], json!([]));
    assert_eq!(snapshot["dropped"], MAX_SAFE_COUNT);
}
