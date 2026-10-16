use std::fmt::Write as _;

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

async fn run_usage_stream(reports: &[Value], done: bool, usage: Value) -> super::Report {
    let directory = tempfile::tempdir().unwrap();
    let content = json!({"id":"statistics","object":"chat.completion.chunk","created":1,
        "model":"real-model","choices":[{"index":0,"delta":{"role":"assistant","content":"hello"},
        "finish_reason":"stop"}]});
    let mut bytes = format!("data: {content}\n\n");
    for usage in reports {
        let event = json!({"id":"statistics","object":"chat.completion.chunk","created":1,
            "model":"real-model","choices":[],"usage":usage});
        writeln!(bytes, "data: {event}\n").unwrap();
    }
    if done {
        bytes.push_str("data: [DONE]\n\n");
    }
    fs::write(directory.path().join("usage.sse"), bytes).unwrap();
    let mut case = json_case();
    case["request"]["mode"] = json!("delete");
    case["upstream"] = json!({"status":200,"headers":{"content-type":["text/event-stream"]},
        "bodyFile":"usage.sse"});
    case["expect"] = if done {
        let mut body = case["expect"]["response"]["body"].clone();
        body.as_object_mut().unwrap().remove("usage");
        json!({"sent":true,"response":{"status":200,"body":body},"stats":{"usage":usage}})
    } else {
        json!({"sent":true,"response":{"status":502},
            "error":{"code":"upstream_protocol","status":502,"phase":"upstream_sse"},
            "stats":{"usage":usage}})
    };
    let path = directory.path().join("usage.case.json");
    fs::write(&path, serde_json::to_vec(&case).unwrap()).unwrap();
    execute(&fixtures("statistics").join("server.toml"), &path)
        .await
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn cumulative_usage_growth_nulls_and_partial_reports_preserve_latest_native_counts() {
    let reports = [
        json!({"prompt_tokens":4,"completion_tokens":1}),
        json!({"prompt_tokens":4,"completion_tokens":3,"total_tokens":7,
            "prompt_tokens_details":{"cached_tokens":1}}),
        Value::Null,
        json!({"prompt_tokens":null,"completion_tokens":6,"total_tokens":10,
            "prompt_tokens_details":{"cached_tokens":2}}),
        json!({"prompt_tokens":5,"total_tokens":11,
            "completion_tokens_details":{"reasoning_tokens":3}}),
    ];
    for done in [false, true] {
        let usage = json!({"inputTokens":5,"outputTokens":6,"totalTokens":11,
            "cachedInputTokens":2,"reasoningOutputTokens":3,"quality":"reported",
            "source":"sse","upstreamCompleted":done});
        let report = run_usage_stream(&reports, done, usage).await;
        assert!(report.passed(), "{}", report.mismatches.join(", "));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_native_usage_affects_quality_without_failing_json_or_sse_requests() {
    for invalid in [
        json!(false),
        json!([]),
        json!("invalid"),
        json!({"total_tokens":-1}),
    ] {
        let mut usage = json!({"inputTokens":null,"outputTokens":null,"totalTokens":null,
            "cachedInputTokens":null,"reasoningOutputTokens":null,"quality":"invalid",
            "source":"sse","upstreamCompleted":true});
        let report = run_usage_stream(std::slice::from_ref(&invalid), true, usage.clone()).await;
        assert!(report.passed(), "{}", report.mismatches.join(", "));

        let mut case = json_case();
        case["upstream"]["body"]["usage"] = invalid;
        usage["source"] = json!("json");
        case["expect"]["stats"]["usage"] = usage;
        let report = run_json_case(&case).await.unwrap();
        assert!(report.passed(), "{}", report.mismatches.join(", "));
    }
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
