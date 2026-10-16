use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use rusqlite::Connection;

use crate::{
    Error, Filter, FinalRecord, GroupBy, Outcome, PendingRecord, RecordState, RequestQuery,
    StorageOptions, UsageQuality, UsageQuery, database, query,
};

fn options() -> StorageOptions {
    StorageOptions {
        dir: PathBuf::from("unused"),
        run_id: "test-run".into(),
        retention_days: 90,
        max_records: 100_000,
        max_db_bytes: 64 * 1024 * 1024,
        generation_concurrency: 32,
        management_concurrency: Some(4),
    }
}

fn deadline() -> Instant { Instant::now() + Duration::from_secs(10) }

fn database() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    database::initialize(&mut connection, true, &options(), deadline()).unwrap();
    connection
}

fn pending(id: &str, started: i64) -> PendingRecord {
    PendingRecord {
        request_id: id.into(),
        run_id: "test-run".into(),
        started_at_ms: started,
        client_stream: true,
        input_bytes: 12,
    }
}

fn terminal(id: &str) -> FinalRecord {
    FinalRecord {
        request_id: id.into(),
        finished_at_ms: 1100,
        outcome: Some(Outcome::Success),
        usage_quality: Some(UsageQuality::Partial),
        input_tokens: Some(0),
        ..FinalRecord::default()
    }
}

fn filter() -> Filter {
    Filter {
        from_ms: 0,
        to_ms: 2000,
        model: None,
        provider: None,
        state: None,
        outcome: None,
    }
}

#[test]
fn terminal_metadata_preserves_unknown_and_real_zero() {
    let mut connection = database();
    database::begin(&mut connection, &pending("one", 1000), &options()).unwrap();
    let before = query::get(&connection, "one").unwrap().unwrap();
    assert_eq!(before.state, RecordState::Pending);
    assert_eq!(before.input_tokens, None);
    database::finish(&connection, &terminal("one")).unwrap();
    let after = query::get(&connection, "one").unwrap().unwrap();
    let json = serde_json::to_value(after).unwrap();
    assert_eq!(json["inputTokens"], "0");
    assert_eq!(json["inputBytes"], "12");
    assert!(json["totalTokens"].is_null());
    assert_eq!(json["startedAt"], "1970-01-01T00:00:01.000Z");
    assert!(matches!(
        database::finish(&connection, &terminal("one")),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        database::finish(&connection, &terminal("absent")),
        Err(Error::Conflict)
    ));
}

#[test]
fn recovery_retains_unknown_without_fabricating_a_terminal_result() {
    let mut connection = database();
    let now = database::now_ms().unwrap();
    database::begin(&mut connection, &pending("one", now), &options()).unwrap();
    database::initialize(&mut connection, false, &options(), deadline()).unwrap();
    let record = query::get(&connection, "one").unwrap().unwrap();
    assert_eq!(record.state, RecordState::Interrupted);
    assert_eq!(record.finished_at_ms, None);
    assert_eq!(record.outcome, None);
    assert_eq!(record.usage_quality, None);
    assert_eq!(record.upstream_completed, None);
}

#[test]
fn schema_mismatch_is_not_recreated() {
    let mut connection = database();
    connection.pragma_update(None, "user_version", 999).unwrap();
    assert!(matches!(
        database::initialize(&mut connection, false, &options(), deadline()),
        Err(Error::Schema)
    ));
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 999);
}

#[test]
fn capacity_evicts_only_terminal_rows_and_never_reuses_ids() {
    let mut connection = database();
    let mut options = options();
    options.max_records = 2;
    database::begin(&mut connection, &pending("active", 900), &options).unwrap();
    database::begin(&mut connection, &pending("old", 1000), &options).unwrap();
    assert!(matches!(
        database::begin(&mut connection, &pending("new", 1100), &options),
        Err(Error::Capacity)
    ));
    database::finish(&connection, &terminal("old")).unwrap();
    database::begin(&mut connection, &pending("new", 1100), &options).unwrap();
    assert!(query::get(&connection, "old").unwrap().is_none());
    assert_eq!(
        query::get(&connection, "active").unwrap().unwrap().state,
        RecordState::Pending
    );
    assert_eq!(
        query::get(&connection, "new").unwrap().unwrap().record_id,
        3
    );
}

#[test]
fn paging_handles_equal_times_and_excludes_new_records() {
    let mut connection = database();
    for id in ["one", "two", "three"] {
        database::begin(&mut connection, &pending(id, 1000), &options()).unwrap();
    }
    let first = query::list(
        &mut connection,
        &RequestQuery {
            filter: filter(),
            limit: 2,
            high_watermark: None,
            before: None,
        },
        deadline(),
    )
    .unwrap();
    assert_eq!(
        first
            .records
            .iter()
            .map(|record| record.request_id.as_str())
            .collect::<Vec<_>>(),
        ["three", "two"]
    );
    database::begin(&mut connection, &pending("four", 900), &options()).unwrap();
    let second = query::list(
        &mut connection,
        &RequestQuery {
            filter: filter(),
            limit: 2,
            high_watermark: Some(first.high_watermark),
            before: first.next,
        },
        deadline(),
    )
    .unwrap();
    assert_eq!(second.records.len(), 1);
    assert_eq!(second.records[0].request_id, "one");
    assert_eq!(second.next, None);
}

#[test]
fn usage_includes_failures_and_unknown_records_with_independent_samples() {
    let mut connection = database();
    for id in ["failed", "pending"] {
        database::begin(&mut connection, &pending(id, 1000), &options()).unwrap();
    }
    let mut final_record = terminal("failed");
    final_record.outcome = Some(Outcome::Failed);
    final_record.output_tokens = Some(7);
    database::finish(&connection, &final_record).unwrap();
    let summary = query::usage(
        &mut connection,
        &UsageQuery {
            filter: filter(),
            group_by: GroupBy::Model,
        },
        &options(),
        deadline(),
    )
    .unwrap();
    let group = &summary.groups[0];
    assert_eq!(group.key, None);
    assert_eq!(group.records, 2);
    assert_eq!(group.states["pending"], 1);
    assert_eq!(group.outcomes["failed"], 1);
    assert_eq!(group.outcomes["unknown"], 1);
    assert_eq!(group.qualities["unknown"], 1);
    assert_eq!(group.tokens.input.sum, Some(0));
    assert_eq!(group.tokens.input.samples, 1);
    assert_eq!(group.tokens.output.sum, Some(7));
    assert_eq!(group.tokens.total.sum, None);
    assert_eq!(group.tokens.total.samples, 0);
}

#[test]
fn usage_sum_exceeds_signed_sql_integer_without_overflow_or_rounding() {
    let mut connection = database();
    connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1025)
      INSERT INTO requests(request_id,run_id,started_at_ms,state,outcome,client_stream,input_tokens,usage_quality)
      SELECT 'r-'||x,'run',1000,'finalized','failed',0,9007199254740991,'partial' FROM n;").unwrap();
    let summary = query::usage(
        &mut connection,
        &UsageQuery {
            filter: filter(),
            group_by: GroupBy::None,
        },
        &options(),
        deadline(),
    )
    .unwrap();
    assert_eq!(
        summary.groups[0].tokens.input.sum,
        Some(9_232_379_236_109_515_775)
    );
    assert_eq!(summary.groups[0].tokens.input.samples, 1025);
}

#[test]
fn broad_group_queries_fail_without_partial_success() {
    let mut connection = database();
    connection
        .execute_batch(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<129)
      INSERT INTO requests(request_id,run_id,started_at_ms,state,client_stream,model)
      SELECT 'r-'||x,'run',1000,'pending',0,'model-'||x FROM n;",
        )
        .unwrap();
    assert!(matches!(
        query::usage(
            &mut connection,
            &UsageQuery {
                filter: filter(),
                group_by: GroupBy::Model
            },
            &options(),
            deadline()
        ),
        Err(Error::TooBroad)
    ));
}

#[test]
fn usage_scan_and_time_window_limits_reject_partial_totals() {
    let mut connection = database();
    connection
        .execute_batch(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100001)
      INSERT INTO requests(request_id,run_id,started_at_ms,state,client_stream)
      SELECT 'r-'||x,'run',1000,'pending',0 FROM n;",
        )
        .unwrap();
    assert!(matches!(
        query::usage(
            &mut connection,
            &UsageQuery {
                filter: filter(),
                group_by: GroupBy::None
            },
            &options(),
            deadline()
        ),
        Err(Error::TooBroad)
    ));
    let mut broad = filter();
    broad.to_ms = 32 * 86_400_000;
    assert!(matches!(
        query::list(
            &mut connection,
            &RequestQuery {
                filter: broad,
                limit: 50,
                high_watermark: None,
                before: None
            },
            deadline()
        ),
        Err(Error::TooBroad)
    ));
}

#[test]
fn invalid_usage_and_expired_queries_do_not_become_valid_data() {
    let mut connection = database();
    database::begin(&mut connection, &pending("one", 1000), &options()).unwrap();
    let mut record = terminal("one");
    record.usage_quality = Some(UsageQuality::Conflicting);
    assert!(matches!(
        database::finish(&connection, &record),
        Err(Error::Invalid)
    ));
    assert_eq!(
        query::get(&connection, "one").unwrap().unwrap().state,
        RecordState::Pending
    );
    assert!(matches!(
        query::usage(
            &mut connection,
            &UsageQuery {
                filter: filter(),
                group_by: GroupBy::None
            },
            &options(),
            Instant::now()
        ),
        Err(Error::Timeout)
    ));
}
