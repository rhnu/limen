use super::{Case, Error, Value, execute, fixtures, fs, group, json};

#[tokio::test(flavor = "current_thread")]
async fn raw_usage_survives_all_transports_policy_edits_and_truncation() {
    group(&fixtures("statistics")).await;
}

fn json_case() -> Value {
    serde_json::from_slice(
        &fs::read(fixtures("statistics").join("json-to-json-raw-usage.case.json")).unwrap(),
    )
    .unwrap()
}

async fn run_json_case(value: &Value) -> Result<super::Report, Error> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("statistics.case.json");
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    execute(&fixtures("statistics").join("server.toml"), &path).await
}

#[tokio::test(flavor = "current_thread")]
async fn wrong_statistics_expectations_report_only_safe_categories() {
    let base = json_case();
    for (category, expectation) in [
        (
            "stats.usage",
            json!({"usage":{"private":"synthetic-private-value"}}),
        ),
        ("stats.hookCalls", json!({"hookCalls":{"onRequest":2}})),
        (
            "stats.timingPresence",
            json!({"timingPresence":{"requestMs":false}}),
        ),
    ] {
        let mut case = base.clone();
        case["expect"] = json!({"stats":expectation});
        let report = run_json_case(&case).await.unwrap();
        assert_eq!(report.mismatches, [category]);
        assert!(!report.passed());
        assert!(
            !report
                .mismatches
                .join(", ")
                .contains("synthetic-private-value")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn absent_stats_groups_are_not_compared_and_absent_stages_count_as_zero() {
    for expected in [
        json!({"sent":true,"stats":{}}),
        json!({"stats":{"hookCalls":{"onUpstreamEvent":0}}}),
        json!({"stats":{"timingPresence":{"upstreamFirstChunkMs":false}}}),
    ] {
        let mut case = json_case();
        case["expect"] = expected;
        assert!(run_json_case(&case).await.unwrap().passed());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn empty_statistics_groups_do_not_supply_an_assertion() {
    for statistics in [
        json!({}),
        json!({"hookCalls":{}}),
        json!({"timingPresence":{}}),
    ] {
        let mut case = json_case();
        case["expect"] = json!({"stats":statistics});
        assert!(matches!(run_json_case(&case).await, Err(Error::Invalid(_))));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_statistics_stages_and_timing_fields_are_rejected() {
    for statistics in [
        json!({"hookCalls":{"unknownStage":1}}),
        json!({"hookCalls":{"onFinish":1}}),
        json!({"timingPresence":{"unknownTimingMs":true}}),
        json!({"timingPresence":{"hooks":true}}),
    ] {
        let mut case = json_case();
        case["expect"] = json!({"sent":true,"stats":statistics});
        assert!(matches!(run_json_case(&case).await, Err(Error::Invalid(_))));
    }
}

#[test]
fn statistics_schema_rejects_unknown_groups_null_and_wrong_scalar_types() {
    for statistics in [
        json!({"unknown":{}}),
        Value::Null,
        json!({"hookCalls":null}),
        json!({"hookCalls":{"onRequest":-1}}),
        json!({"hookCalls":{"onRequest":"1"}}),
        json!({"timingPresence":null}),
        json!({"timingPresence":{"requestMs":1}}),
    ] {
        let mut case = json_case();
        case["expect"] = json!({"stats":statistics});
        assert!(serde_json::from_value::<Case>(case).is_err());
    }
}
