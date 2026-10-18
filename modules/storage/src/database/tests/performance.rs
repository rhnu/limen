use super::*;
use crate::{
    GenerationFacts, PerformanceOutcome, PhaseExecutor, PhaseScope, PhaseTiming, RequestTimings,
    TimingSample,
};

fn phase() -> PhaseTiming {
    PhaseTiming {
        phase: "beforeSend".into(),
        scope: PhaseScope::Provider,
        executor: PhaseExecutor::Script,
        calls: 2,
        failed_calls: 0,
        interrupted_calls: 0,
        reported_calls: 2,
        skipped_budget_calls: 0,
        unavailable_calls: 0,
        wall_us: Some(250),
        queue: TimingSample {
            total_us: Some(0),
            samples: 2,
        },
        initial_call: TimingSample {
            total_us: Some(100),
            samples: 1,
        },
        pending: TimingSample::default(),
        input_bridge: TimingSample::default(),
        output_bridge: TimingSample::default(),
    }
}

#[test]
fn precision_parameters_and_phase_samples_round_trip_without_fabricated_zero() {
    let mut connection = database();
    let mut initial = pending("precise", 1000);
    initial.client_parameters = crate::project_generation_parameters(
        &serde_json::json!({"temperature":null,"stream":false,"tools":[{}]}),
    );
    database::begin(&mut connection, &initial, &options()).unwrap();
    let mut final_record = terminal("precise");
    final_record.timings = RequestTimings {
        request_us: Some(1234),
        storage_begin_us: Some(15),
        cleanup_us: Some(42),
        ..RequestTimings::default()
    };
    final_record.phases = vec![phase()];
    final_record.upstream_parameters = Some(crate::project_generation_parameters(
        &serde_json::json!({"temperature":0.5,"stream":true}),
    ));
    database::finish(&connection, &final_record).unwrap();
    let detail = query::get(&connection, "precise").unwrap().unwrap();
    assert_eq!(detail.request_ms, Some(1.234));
    assert_eq!(detail.timings.cleanup_us, Some(42));
    assert_eq!(detail.phases[0].queue.total_us, Some(0));
    assert_eq!(detail.phases[0].queue.samples, 2);
    assert_eq!(detail.phases[0].pending.total_us, None);
    assert_eq!(detail.phases[0].pending.samples, 0);
    assert_eq!(detail.parameters.client, initial.client_parameters);
    assert_eq!(detail.parameters.upstream, final_record.upstream_parameters);
    let list = query::list(
        &mut connection,
        &RequestQuery {
            filter: filter(),
            limit: 10,
            high_watermark: None,
            before: None,
        },
        deadline(),
    )
    .unwrap();
    let summary = serde_json::to_value(&list.records[0]).unwrap();
    assert!(summary.get("parameters").is_none());
    assert!(summary.get("phases").is_none());
}

#[test]
fn finalization_is_atomic_even_when_a_child_insert_fails() {
    let mut connection = database();
    database::begin(&mut connection, &pending("atomic", 1000), &options()).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_phase BEFORE INSERT ON request_phases BEGIN SELECT RAISE(ABORT,'test rejection'); END").unwrap();
    let mut final_record = terminal("atomic");
    final_record.phases = vec![phase()];
    assert!(database::finish(&connection, &final_record).is_err());
    let detail = query::get(&connection, "atomic").unwrap().unwrap();
    assert_eq!(detail.state, RecordState::Pending);
    assert!(detail.finished_at_ms.is_none());
    assert!(detail.phases.is_empty());
    assert!(detail.timings.request_us.is_none());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM request_timings", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TRIGGER reject_phase")
        .unwrap();
    database::finish(&connection, &final_record).unwrap();
}

#[test]
fn eviction_removes_all_details_and_duplicate_phase_keys_are_rejected() {
    let mut connection = database();
    let mut options = options();
    options.max_records = 1;
    database::begin(&mut connection, &pending("old", 1000), &options).unwrap();
    let mut final_record = terminal("old");
    final_record.phases = vec![phase(), phase()];
    assert!(matches!(
        database::finish(&connection, &final_record),
        Err(Error::Invalid)
    ));
    final_record.phases.pop();
    database::finish(&connection, &final_record).unwrap();
    database::begin(&mut connection, &pending("new", 1100), &options).unwrap();
    for table in ["request_timings", "request_phases"] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn performance_cohort_percentiles_and_overall_are_computed_from_request_samples() {
    let mut connection = database();
    for index in 1..=21 {
        let id = format!("p-{index}");
        database::begin(&mut connection, &pending(&id, 1000), &options()).unwrap();
        let mut record = terminal(&id);
        record.model = Some(if index == 1 { "small" } else { "large" }.into());
        record.timings.request_us = Some(index * 1000);
        record.output_tokens = Some(1);
        record.phases = vec![phase()];
        if index == 21 {
            record.outcome = Some(Outcome::Failed);
        }
        database::finish(&connection, &record).unwrap();
    }
    let mut request = UsageQuery {
        filter: filter(),
        group_by: GroupBy::Model,
        performance_outcome: PerformanceOutcome::Success,
    };
    let summary = query::usage(&mut connection, &request, &options(), deadline()).unwrap();
    let metric = &summary.overall.performance.request_ms;
    assert_eq!(metric.samples, 20);
    assert_eq!(metric.mean, Some(10.5));
    assert_eq!(metric.p50, Some(10.0));
    assert_eq!(metric.p95, Some(19.0));
    assert_eq!(metric.p99, Some(20.0));
    assert_eq!(summary.overall.tokens.output.sum, Some(21));
    assert_eq!(summary.overall.records, 21);
    assert_eq!(summary.overall.phases[0].wall.sum_ms, Some(5.0));
    assert_eq!(summary.overall.phases[0].wall.samples, 20);
    assert_eq!(summary.overall.phases[0].queue.sum_ms, Some(0.0));
    assert_eq!(summary.overall.phases[0].queue.samples, 40);
    assert_eq!(summary.overall.phases[0].pending.sum_ms, None);
    request.performance_outcome = PerformanceOutcome::Failed;
    let failed = query::usage(&mut connection, &request, &options(), deadline()).unwrap();
    assert_eq!(failed.overall.performance.request_ms.samples, 1);
    assert_eq!(failed.overall.performance.request_ms.mean, Some(21.0));
    assert_eq!(failed.overall.tokens.output.sum, Some(21));
    request.performance_outcome = PerformanceOutcome::Cancelled;
    let empty = query::usage(&mut connection, &request, &options(), deadline()).unwrap();
    assert_eq!(empty.overall.performance.request_ms.samples, 0);
    assert_eq!(empty.overall.performance.request_ms.mean, None);
}

#[test]
fn buffered_delivery_is_excluded_from_native_ttft_but_not_known_json_rate() {
    let mut connection = database();
    database::begin(&mut connection, &pending("buffered", 1000), &options()).unwrap();
    let mut record = terminal("buffered");
    record.usage_source = Some("json".into());
    record.upstream_media = Some("json".into());
    record.upstream_completed = Some(true);
    record.output_tokens = Some(2);
    record.timings.upstream_total_us = Some(500_000);
    record.generation = GenerationFacts {
        downstream_first_content_us: Some(600_000),
        downstream_first_text_us: Some(600_000),
        downstream_buffered: true,
        ..GenerationFacts::default()
    };
    database::finish(&connection, &record).unwrap();
    let detail = query::get(&connection, "buffered").unwrap().unwrap();
    assert_eq!(detail.metrics.downstream_ttft_ms, Some(600.0));
    let mut filter = filter();
    filter.client_stream = Some(true);
    filter.upstream_media = Some("json".into());
    let result = query::usage(
        &mut connection,
        &UsageQuery {
            filter,
            group_by: GroupBy::None,
            performance_outcome: PerformanceOutcome::Success,
        },
        &options(),
        deadline(),
    )
    .unwrap();
    assert_eq!(result.overall.performance.downstream_ttft_ms.samples, 0);
    assert_eq!(
        result.overall.performance.downstream_first_text_ms.samples,
        0
    );
    assert_eq!(
        result.overall.performance.average_output_tps.mean,
        Some(4.0)
    );
}
