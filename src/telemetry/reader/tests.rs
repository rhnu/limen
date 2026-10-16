use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::json;

use super::{Cursor, LogReader, Query, ReadError, SCAN_BYTES, URL_SAFE_NO_PAD, scan};

fn record(id: &str, level: &str, request: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({"schemaVersion":1,"timestamp":"2026-10-16T00:00:00.000Z","level":level,"logId":id,"event":"requestFinished","requestId":request})).unwrap();
    bytes.push(b'\n');
    bytes
}

fn deadline() -> Instant { Instant::now() + Duration::from_secs(10) }

#[test]
fn reverse_pagination_preserves_unreturned_matching_record() {
    let one = record("log-1", "info", "request-1");
    let two = record("log-2", "warn", "request-2");
    let three = record("log-3", "error", "request-1");
    let bytes = [one, two, three].concat();
    let page = scan(&bytes, 0, 1, None, None, deadline()).unwrap();
    assert_eq!(page.data[0]["logId"], "log-3");
    assert!(page.offset > 0);
    let second = scan(
        &bytes[..usize::try_from(page.offset).unwrap()],
        0,
        1,
        None,
        None,
        deadline(),
    )
    .unwrap();
    assert_eq!(second.data[0]["logId"], "log-2");
    let last = scan(
        &bytes[..usize::try_from(second.offset).unwrap()],
        0,
        1,
        None,
        None,
        deadline(),
    )
    .unwrap();
    assert_eq!(last.data[0]["logId"], "log-1");
    assert_eq!(last.offset, 0);
}

#[test]
fn filters_and_corrupt_tail_do_not_expose_raw_text() {
    let bytes = [
        record("log-1", "info", "r-1"),
        record("log-2", "warn", "r-2"),
        record("log-3", "error", "r-1"),
        b"secret-corrupt\n{\"half\":".to_vec(),
    ]
    .concat();
    let page = scan(&bytes, 0, 100, Some("warn"), Some("r-1"), deadline()).unwrap();
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0]["logId"], "log-3");
    assert_eq!(page.skipped, 2);
    assert_eq!(page.offset, 0);
}

#[test]
fn unmatched_scan_window_retains_cursor_and_timeout_fails() {
    let bytes = record("log-1", "debug", "r-1");
    let page = scan(&bytes, 500, 100, Some("error"), None, deadline()).unwrap();
    assert_eq!(page.data, Vec::<serde_json::Value>::new());
    assert!(page.offset > 0);
    assert!(matches!(
        scan(&bytes, 0, 100, None, None, Instant::now()),
        Err(ReadError::Timeout)
    ));
    let garbage = vec![b'x'; SCAN_BYTES];
    let page = scan(&garbage, 100, 100, None, None, deadline()).unwrap();
    assert_eq!(page.offset, 100);
    assert_eq!(page.skipped, 1);
}

#[test]
fn strict_queries_and_restart_bound_cursors() {
    assert!(Query::parse(&[("limit".into(), "0".into())]).is_err());
    assert!(Query::parse(&[("path".into(), "secret".into())]).is_err());
    assert!(Query::parse(&[("minLevel".into(), "INFO".into())]).is_err());
    assert!(Query::parse(&[("limit".into(), "1".into()), ("limit".into(), "2".into())]).is_err());
    let cursor = Cursor {
        version: 1,
        run_id: "old-run".into(),
        file_id: "log-1".into(),
        offset: 100,
        size: 200,
        min_level: None,
        request_id: None,
    };
    let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).unwrap());
    let query = Query::parse(&[("cursor".into(), encoded)]).unwrap();
    let reader = LogReader::new("unused".into(), 8, "new-run".into());
    assert!(matches!(
        reader.cursor(&query),
        Err(ReadError::CursorExpired)
    ));
}

#[test]
fn parsed_logs_project_only_approved_metadata() {
    let mut record: serde_json::Value =
        serde_json::from_slice(&record("log-1", "info", "r-1")).unwrap();
    record["body"] = json!("secret-body");
    record["authorization"] = json!("secret-token");
    let mut bytes = serde_json::to_vec(&record).unwrap();
    bytes.push(b'\n');
    let page = scan(&bytes, 0, 100, None, None, deadline()).unwrap();
    assert_eq!(page.data.len(), 1);
    assert!(page.data[0].get("body").is_none());
    assert!(page.data[0].get("authorization").is_none());
}

#[test]
fn response_budget_keeps_every_unreturned_record_on_next_page() {
    let mut bytes = Vec::new();
    for index in 0..100 {
        let mut value: serde_json::Value =
            serde_json::from_slice(&record(&format!("log-{index}"), "info", "r-1")).unwrap();
        value["model"] = json!("x".repeat(12_000));
        bytes.extend(serde_json::to_vec(&value).unwrap());
        bytes.push(b'\n');
    }
    let first = scan(&bytes, 0, 200, None, None, deadline()).unwrap();
    assert!(first.data.len() < 100);
    let second = scan(
        &bytes[..usize::try_from(first.offset).unwrap()],
        0,
        200,
        None,
        None,
        deadline(),
    )
    .unwrap();
    assert_eq!(first.data.len() + second.data.len(), 100);
    assert_eq!(first.data[0]["logId"], "log-99");
    assert_eq!(second.data.last().unwrap()["logId"], "log-0");
    assert_eq!(second.offset, 0);
}
