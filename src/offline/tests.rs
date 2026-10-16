use std::{
    fs,
    path::{Path, PathBuf},
};

use limen_config::{ResourceLimits, load_server};
use serde_json::{Value, json};

use super::{Error, Report, discover, header_map, read, run_case, schema::Case, stream};
use crate::bootstrap::Bootstrap;

mod observation_output;
mod policies;
mod statistics;

fn fixtures(group: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/gateway")
        .join(group)
}

/// Every case has a fresh VM; cleanup happens before either errors or mismatches are asserted.
async fn execute(server: &Path, case: &Path) -> Result<Report, Error> {
    let (path, settings) = load_server(Some(server)).expect("valid test server settings");
    let loaded = Bootstrap::start(&path, settings)
        .await
        .expect("fixture configuration starts");
    let result = run_case(&loaded, case).await;
    let cleanup = loaded.shutdown().await;
    assert!(cleanup.is_ok(), "fixture runtime did not shut down");
    result
}

async fn group(directory: &Path) {
    for case in discover(directory).expect("discover fixture cases") {
        let report = execute(&directory.join("server.toml"), &case)
            .await
            .expect("valid fixture runs");
        assert!(
            report.passed(),
            "case {}: {}",
            report.name,
            report.mismatches.join(", ")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn provider_model_and_client_policy() { group(&fixtures("policy")).await; }

#[tokio::test(flavor = "current_thread")]
async fn ordered_pipeline_and_failed_hook_commits() { group(&fixtures("pipeline")).await; }

#[tokio::test(flavor = "current_thread")]
async fn upstream_drop_split_and_outgoing_stream_policy() { group(&fixtures("streams")).await; }

#[tokio::test(flavor = "current_thread")]
async fn transport_intent_facts_tools_usage_and_truncation() {
    group(&fixtures("transport")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn upstream_failures_cannot_become_success() { group(&fixtures("invalid-status")).await; }

#[tokio::test(flavor = "current_thread")]
async fn observer_failures_preserve_readonly_state_and_terminal_results() {
    group(&fixtures("observations")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn case_lifetimes_isolate_lexical_and_request_state() { group(&fixtures("isolation")).await; }

#[tokio::test(flavor = "current_thread")]
async fn business_rejection_and_client_context_follow_production_pipeline() {
    group(&fixtures("script-policy")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn runnable_examples_cover_all_four_transports() {
    group(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/testing")).await;
}

fn minimal_case() -> Value {
    json!({
        "name":"minimal",
        "request":{"model":"chat","messages":[{"role":"user","content":"hello"}]},
        "upstream":{"status":200,"headers":{"content-type":["application/json"]},"body":{"model":"real-model"}},
        "expect":{"sent":true}
    })
}

async fn run_value(value: &Value) -> Result<Report, Error> {
    let directory = tempfile::tempdir().expect("temporary case directory");
    let path = directory.path().join("input.case.json");
    fs::write(&path, serde_json::to_vec(value).unwrap()).expect("write test case");
    execute(&fixtures("policy").join("server.toml"), &path).await
}

#[tokio::test(flavor = "current_thread")]
async fn deliberate_wrong_expectations_fail_by_safe_category() {
    let base: Value =
        serde_json::from_slice(&fs::read(fixtures("policy").join("defaults.case.json")).unwrap())
            .unwrap();
    let mutations = [
        ("sent", json!({"sent":false})),
        (
            "outbound.body",
            json!({"outbound":{"body":{"secret":"synthetic-body-secret"}}}),
        ),
        (
            "outbound.url",
            json!({"outbound":{"url":"https://wrong.invalid/?synthetic-query-secret"}}),
        ),
        (
            "outbound.headers",
            json!({"outbound":{"headers":{"authorization":["synthetic-upstream-secret"]}}}),
        ),
        ("response.status", json!({"response":{"status":201}})),
        (
            "response.headers",
            json!({"response":{"headers":{"set-cookie":["synthetic-cookie-secret"]}}}),
        ),
        (
            "response.body",
            json!({"response":{"body":{"secret":"synthetic-body-secret"}}}),
        ),
        ("state", json!({"state":{"wrong":"synthetic-body-secret"}})),
        (
            "error",
            json!({"sent":true,"error":{"code":"wrong","status":500,"phase":"beforeSend"}}),
        ),
        (
            "observationFailed",
            json!({"sent":true,"observationFailed":true}),
        ),
    ];
    for (category, expected) in mutations {
        let mut value = base.clone();
        value["expect"] = expected;
        let report = run_value(&value)
            .await
            .expect("mismatch is a report, not an execution error");
        assert_eq!(report.mismatches, [category]);
        assert!(!report.passed());
        let output = report.mismatches.join(", ");
        assert!(!output.contains("synthetic-"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn wrong_stream_chunks_and_done_are_detected() {
    let directory = fixtures("transport");
    let source = directory.join("json-to-sse.case.json");
    let original: Value = serde_json::from_slice(&fs::read(source).unwrap()).unwrap();
    for (category, expected) in [
        ("response.events", json!({"events":[]})),
        ("response.done", json!({"done":false})),
    ] {
        let mut value = original.clone();
        value["expect"] = json!({"response":expected});
        // This case uses a JSON body, so moving its expectations does not alter bodyFile resolution.
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("wrong.case.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let report = execute(&directory.join("server.toml"), &path)
            .await
            .unwrap();
        assert_eq!(report.mismatches, [category]);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn undeclared_core_and_observer_failures_are_mismatches() {
    for (group, case, field, category) in [
        ("pipeline", "failed-throw", "error", "error"),
        (
            "observations",
            "throw",
            "observationFailed",
            "observationFailed",
        ),
    ] {
        let directory = fixtures(group);
        let mut value: Value =
            serde_json::from_slice(&fs::read(directory.join(format!("{case}.case.json"))).unwrap())
                .unwrap();
        value["expect"].as_object_mut().unwrap().remove(field);
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("failure.case.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let report = execute(&directory.join("server.toml"), &path)
            .await
            .unwrap();
        assert_eq!(report.mismatches, [category]);
    }
}

#[test]
fn schema_rejects_unknown_fields_and_wrong_types() {
    for pointer in ["", "/upstream", "/expect"] {
        let mut value = minimal_case();
        value.pointer_mut(pointer).unwrap()["unexpected"] = json!("synthetic-body-secret");
        let result = serde_json::from_value::<Case>(value);
        assert!(result.is_err());
        let error = Error::Json(result.err().unwrap());
        assert!(!error.to_string().contains("synthetic-"));
    }
    for (pointer, replacement) in [
        ("/clientHeaders", json!({"x-test":"one"})),
        ("/upstream/status", json!("200")),
        ("/upstream/headers", json!({"x-test":"one"})),
        ("/expect/response", json!({"done":"true"})),
    ] {
        let mut value = minimal_case();
        if pointer == "/expect/response" {
            value["expect"]["response"] = replacement;
        } else if pointer == "/clientHeaders" {
            value["clientHeaders"] = replacement;
        } else {
            *value.pointer_mut(pointer).unwrap() = replacement;
        }
        assert!(serde_json::from_value::<Case>(value).is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn semantic_fixture_validation_has_safe_errors() {
    let mutations = [
        ("/name", json!("unsafe\nsynthetic-body-secret")),
        ("/name", json!("")),
        ("/upstream/status", json!(99)),
        ("/upstream/bodyFile", json!("synthetic-body-secret")),
        (
            "/upstream/headers",
            json!({"bad\nname":["synthetic-upstream-secret"]}),
        ),
        ("/expect", json!({})),
    ];
    for (pointer, replacement) in mutations {
        let mut value = minimal_case();
        if pointer == "/upstream/bodyFile" {
            value["upstream"]["bodyFile"] = replacement;
        } else {
            *value.pointer_mut(pointer).unwrap() = replacement;
        }
        let error = run_value(&value)
            .await
            .err()
            .expect("invalid fixture is rejected");
        assert!(matches!(error, Error::Invalid(_) | Error::HeaderName(_)));
        assert!(!error.to_string().contains("synthetic-"));
    }
}

#[tokio::test(flavor = "current_thread")]
async fn client_fixture_headers_are_bounded_and_cannot_override_runner_fields() {
    for headers in [
        json!({"Authorization":["Bearer synthetic-private-value"]}),
        json!({"Content-Type":["text/plain"]}),
        json!({"x-empty":[]}),
        json!({"x-value":["bad\nsynthetic-private-value"]}),
        json!({"x-many":vec!["v"; 100]}),
        json!({"x-large":["v".repeat(32 * 1024)]}),
    ] {
        let mut value = minimal_case();
        value["clientHeaders"] = headers;
        let error = run_value(&value)
            .await
            .err()
            .expect("invalid client fixture headers must fail");
        assert!(matches!(error, Error::Invalid(_) | Error::HeaderValue(_)));
        assert!(!error.to_string().contains("synthetic-private-value"));
    }
}

#[test]
fn bounded_fixture_reads_accept_limit_and_reject_one_more_byte() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bytes");
    fs::write(&path, b"1234").unwrap();
    assert_eq!(read(&path, 4).unwrap(), b"1234");
    let error = read(&path, 3).unwrap_err();
    assert!(matches!(
        error,
        Error::Invalid("fixture exceeds byte limit")
    ));
}

#[test]
fn header_validation_is_case_insensitive_and_preserves_multiple_values() {
    let fields = serde_json::from_value(json!({"X-Test":["one","two"]})).unwrap();
    let map = header_map(&fields).unwrap();
    let values = map
        .get_all("x-test")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values, ["one", "two"]);
    for value in [
        json!({"X-Test":["one"],"x-test":["two"]}),
        json!({"bad name":["one"]}),
        json!({"x-test":["bad\nvalue"]}),
    ] {
        assert!(header_map(&serde_json::from_value(value).unwrap()).is_err());
    }
}

#[test]
fn excessive_header_values_return_an_error_instead_of_panicking() {
    let fields = (0..40_000)
        .map(|index| (format!("x-field-{index}"), vec!["one".to_owned()]))
        .collect();
    assert!(matches!(header_map(&fields), Err(Error::HeaderCapacity(_))));
}

#[tokio::test(flavor = "current_thread")]
async fn same_runtime_keeps_failed_and_concurrent_request_states_separate() {
    let directory = fixtures("pipeline");
    let (path, settings) = load_server(Some(&directory.join("server.toml"))).unwrap();
    let loaded = Bootstrap::start(&path, settings).await.unwrap();
    let failed = run_case(&loaded, &directory.join("failed-throw.case.json")).await;
    let first_path = directory.join("concurrent-first.case.json");
    let second_path = directory.join("concurrent-second.case.json");
    let (first, second) = tokio::join!(
        run_case(&loaded, &first_path),
        run_case(&loaded, &second_path)
    );
    let clean = run_case(&loaded, &directory.join("clean-after-failure.case.json")).await;
    let cleanup = loaded.shutdown().await;
    assert!(cleanup.is_ok());
    for result in [failed, first, second, clean] {
        let report = result.unwrap();
        assert!(
            report.passed(),
            "case {}: {}",
            report.name,
            report.mismatches.join(", ")
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn observer_cleanup_runs_once_without_recursion_and_shares_its_budget() {
    let directory = fixtures("observer-sequence");
    for (initial, after) in [
        ("bad", "after-bad"),
        ("error-wait", "after-error-wait"),
        ("finish-wait", "after-finish-wait"),
    ] {
        let (path, settings) = load_server(Some(&directory.join("server.toml"))).unwrap();
        let loaded = Bootstrap::start(&path, settings).await.unwrap();
        let first = run_case(&loaded, &directory.join(format!("{initial}.case.json"))).await;
        let second = run_case(&loaded, &directory.join(format!("{after}.case.json"))).await;
        let cleanup = loaded.shutdown().await;
        assert!(cleanup.is_ok());
        for result in [first, second] {
            let report = result.unwrap();
            assert!(
                report.passed(),
                "case {}: {}",
                report.name,
                report.mismatches.join(", ")
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn json_responses_cannot_pass_empty_sse_assertions() {
    let mut value = minimal_case();
    value["expect"] = json!({"response":{"events":[],"done":false}});
    let report = run_value(&value).await.unwrap();
    assert_eq!(report.mismatches, ["response.events", "response.done"]);
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_null_assertions_are_not_treated_as_missing() {
    for (expected, category) in [
        (json!({"response":{"body":null}}), "response.body"),
        (json!({"outbound":{"body":null}}), "outbound.body"),
        (json!({"state":null}), "state"),
    ] {
        let mut value = minimal_case();
        value["expect"] = expected;
        let report = run_value(&value).await.unwrap();
        assert_eq!(report.mismatches, [category]);
    }
    let mut value = minimal_case();
    value["upstream"]["body"] = Value::Null;
    value["expect"] = json!({"sent":true,"error":{"code":"upstream_protocol","status":502,"phase":"upstream_body"}});
    assert!(run_value(&value).await.unwrap().passed());
}

#[test]
fn typed_optional_assertions_reject_explicit_null() {
    for expected in [
        json!({"sent":null}),
        json!({"response":{"done":null}}),
        json!({"error":null}),
        json!({"observationFailed":null}),
    ] {
        let mut value = minimal_case();
        value["expect"] = expected;
        assert!(serde_json::from_value::<Case>(value).is_err());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn case_json_and_body_file_limits_apply_before_request_execution() {
    for limit in ["source", "depth", "nodes", "body"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.case.json");
        let mut value = minimal_case();
        if limit == "body" {
            value["upstream"].as_object_mut().unwrap().remove("body");
            value["upstream"]["bodyFile"] = json!("large.sse");
            fs::write(directory.path().join("large.sse"), b"123456789").unwrap();
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (config, settings) =
            load_server(Some(&fixtures("policy").join("server.toml"))).unwrap();
        let mut loaded = Bootstrap::start(&config, settings).await.unwrap();
        match limit {
            "source" => loaded.server.limits.max_source_bytes = 16,
            "depth" => loaded.server.limits.max_json_depth = 1,
            "nodes" => loaded.server.limits.max_json_nodes = 1,
            "body" => {
                loaded.server.limits.max_output_bytes = 8;
                loaded.server.limits.max_aggregate_bytes = 8;
            },
            _ => unreachable!(),
        }
        let result = run_case(&loaded, &path).await;
        let cleanup = loaded.shutdown().await;
        assert!(cleanup.is_ok());
        let error = result.err().expect("bounded input must fail");
        if matches!(limit, "depth" | "nodes") {
            assert!(matches!(error, Error::JsonLimits(_)));
        } else {
            assert!(matches!(
                error,
                Error::Invalid("fixture exceeds byte limit")
            ));
        }
    }
}

#[test]
fn parsed_sse_obeys_event_count_and_byte_limits() {
    let bytes = b"data: {\"v\":1}\n\ndata: {\"v\":2}\n\ndata: [DONE]\n\n";
    let limits = ResourceLimits::default();
    assert_eq!(
        stream(bytes, &limits).unwrap(),
        (vec![json!({"v":1}), json!({"v":2})], true)
    );
    let limits = ResourceLimits {
        max_aggregate_events: 1,
        ..limits
    };
    assert!(matches!(
        stream(bytes, &limits),
        Err(Error::Invalid("offline SSE exceeds event limit"))
    ));
    let huge = format!("data: {{\"text\":\"{}\"}}\n\n", "x".repeat(2048));
    let limits = ResourceLimits {
        max_event_bytes: 1024,
        ..limits
    };
    assert!(matches!(
        stream(huge.as_bytes(), &limits),
        Err(Error::Invalid("offline SSE event exceeds byte limit"))
    ));
}

#[test]
fn discovery_selects_cases_in_order_and_rejects_empty_directories() {
    let directory = tempfile::tempdir().unwrap();
    assert!(matches!(
        discover(directory.path()),
        Err(Error::Invalid("no .case.json files found"))
    ));
    for name in ["b.case.json", "a.case.json", "ignore.json"] {
        fs::write(directory.path().join(name), b"{}").unwrap();
    }
    fs::create_dir(directory.path().join("nested.case.json")).unwrap();
    let cases = discover(directory.path()).unwrap();
    assert_eq!(
        cases,
        [
            directory.path().join("a.case.json"),
            directory.path().join("b.case.json")
        ]
    );
    assert_eq!(discover(&cases[0]).unwrap(), [cases[0].clone()]);
}
