use std::fmt::Write;

use super::{Error, Report, Value, execute, fixtures, fs, group, json};

fn fixture(name: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixtures("observation-output").join(format!("{name}.case.json"))).unwrap(),
    )
    .unwrap()
}

async fn run_value(value: &Value) -> Result<Report, Error> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("observation.case.json");
    let mut value = value.clone();
    if let Some(relative) = value["upstream"]["bodyFile"].as_str() {
        value["upstream"]["bodyFile"] = json!(fixtures("observation-output").join(relative));
    }
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    execute(&fixtures("observation-output").join("server.toml"), &path).await
}

#[tokio::test(flavor = "current_thread")]
async fn observations_follow_commits_order_limits_and_observer_outcomes() {
    group(&fixtures("observation-output")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn vm_failures_and_invalid_request_drafts_discard_events_and_dropped_counts() {
    for mode in ["promise", "cycle", "url", "header"] {
        let mut case = fixture("throw-discards-current-hook");
        case["request"]["mode"] = json!(mode);
        case["expect"]["state"]["mode"] = json!(mode);
        let report = run_value(&case).await.unwrap();
        assert!(report.passed(), "{mode}: {:?}", report.mismatches);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_status_discards_successful_vm_observations_and_state() {
    let mut case = fixture("normal-provider-model-and-observer");
    case["request"]["mode"] = json!("status");
    case["expect"] = json!({
        "sent":true,
        "response":{"status":500},
        "state":{"mode":"status","committed":"model"},
        "error":{"code":"hook_failed","status":500,"phase":"afterReceive"},
        "observations":{"events":[
            {"event":"request","phase":"onRequest","step":null,"count":1},
            {"event":"provider","phase":"beforeSend","step":"provider","count":1},
            {"event":"model","phase":"beforeSend","step":"model","count":1},
            {"event":"error","phase":"onError","step":null,"count":1},
            {"event":"finish","phase":"onFinish","step":null,"count":1}
        ],"dropped":0}
    });
    let report = run_value(&case).await.unwrap();
    assert!(report.passed(), "{:?}", report.mismatches);
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_chat_event_keeps_prior_provider_observation_and_discards_model_draft() {
    let mut case = fixture("sse-hooks-aggregate-observations");
    case["request"]["mode"] = json!("event");
    case["expect"] = json!({
        "sent":true,
        "response":{"status":500},
        "state":{"mode":"event","committed":"model"},
        "error":{"code":"hook_failed","status":500,"phase":"onUpstreamEvent"},
        "observations":{"events":[
            {"event":"request","phase":"onRequest","step":null,"count":1},
            {"event":"provider","phase":"beforeSend","step":"provider","count":1},
            {"event":"model","phase":"beforeSend","step":"model","count":1},
            {"event":"response","phase":"afterReceive","step":"provider","count":1},
            {"event":"upstream","phase":"onUpstreamEvent","step":"provider","count":1},
            {"event":"error","phase":"onError","step":null,"count":1},
            {"event":"finish","phase":"onFinish","step":null,"count":1}
        ],"dropped":0}
    });
    let report = run_value(&case).await.unwrap();
    assert!(report.passed(), "{:?}", report.mismatches);
}

#[tokio::test(flavor = "current_thread")]
async fn wrong_observation_expectations_have_safe_mismatch_categories() {
    let mut case = fixture("normal-provider-model-and-observer");
    case["expect"] = json!({"observations":{"events":[{
        "event":"synthetic-private-value","phase":"beforeSend","step":null,"count":999
    }],"dropped":0}});
    let report = run_value(&case).await.unwrap();
    assert_eq!(report.mismatches, ["observations"]);
    assert!(!report.passed());
    assert!(
        !report
            .mismatches
            .join(", ")
            .contains("synthetic-private-value")
    );
}

fn stream_chunk(choices: Value, model: &str) -> Value {
    let mut event = json!({"id":"commit", "object":"chat.completion.chunk", "created":1,
        "model":model});
    event["choices"] = choices;
    event
}

fn stream_choice(index: u64, content: &str, finish: Option<&str>) -> Value {
    json!({"index":index, "delta":{"role":"assistant", "content":content},
        "finish_reason":finish})
}

async fn stream_commit_case(mode: &str, events: &[Value], expected: Value) -> Report {
    let directory = tempfile::tempdir().unwrap();
    let upstream = directory.path().join("upstream.sse");
    let mut body = String::new();
    for event in events {
        write!(body, "data: {event}\n\n").unwrap();
    }
    body.push_str("data: [DONE]\n\n");
    fs::write(&upstream, body).unwrap();
    let case = json!({"name":mode,
        "request":{"model":if mode.starts_with("provider-") { "provider" } else { "chat" },
            "messages":[{}], "stream":true, "mode":mode},
        "upstream":{"status":200, "headers":{"content-type":["text/event-stream"]},
            "bodyFile":upstream},
        "expect":expected});
    let path = directory.path().join("commit.case.json");
    fs::write(&path, serde_json::to_vec(&case).unwrap()).unwrap();
    execute(&fixtures("stream-commit").join("server.toml"), &path)
        .await
        .unwrap()
}

fn stream_commit_observations(provider: u64, model: u64) -> Value {
    let mut events = Vec::new();
    if provider > 0 {
        events.push(json!({"event":"provider", "phase":"onStreamEvent",
            "step":"provider", "count":provider}));
    }
    if model > 0 {
        events.push(json!({"event":"model", "phase":"onStreamEvent",
            "step":"model", "count":model}));
    }
    events.push(json!({"event":"finish", "phase":"onFinish", "step":null, "count":1}));
    json!({"events":events, "dropped":model})
}

#[tokio::test(flavor = "current_thread")]
async fn stream_commit_rejects_framing_and_invalid_batch_tails_before_effects_or_output() {
    let event = stream_chunk(json!([stream_choice(0, "hello", Some("stop"))]), "upstream");
    let mut failures = Vec::new();
    for mode in ["framing", "batch-finished", "batch-framing"] {
        let report = stream_commit_case(
            mode,
            std::slice::from_ref(&event),
            json!({
                "response":{"status":500},
                "state":{"mode":mode, "providerCalls":1},
                "error":{"code":"hook_failed", "status":500, "phase":"onStreamEvent"},
                "observations":stream_commit_observations(1, 0)
            }),
        )
        .await;
        if !report.passed() {
            failures.push((mode, report.mismatches));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn stream_commit_retains_earlier_calls_when_a_choice_is_already_finished() {
    let events = [
        stream_chunk(json!([stream_choice(0, "hello", None)]), "upstream"),
        stream_chunk(json!([stream_choice(0, "later", Some("stop"))]), "upstream"),
    ];
    let report = stream_commit_case(
        "finished-choice",
        &events,
        json!({
            "response":{"status":200, "events":[stream_chunk(
                json!([stream_choice(0, "hello", Some("stop"))]), "chat")], "done":false},
            "state":{"mode":"finished-choice", "providerCalls":2, "modelCalls":1},
            "error":{"code":"hook_failed", "status":500, "phase":"onStreamEvent"},
            "observations":stream_commit_observations(2, 1)
        }),
    )
    .await;
    assert!(report.passed(), "{:?}", report.mismatches);
}

#[tokio::test(flavor = "current_thread")]
async fn stream_commit_accepts_a_complete_multi_choice_split_batch() {
    let first = stream_choice(0, "one", Some("stop"));
    let second = stream_choice(1, "two", Some("length"));
    let event = stream_chunk(json!([first, second]), "upstream");
    let report = stream_commit_case(
        "split",
        &[event],
        json!({
            "response":{"status":200, "events":[
                stream_chunk(json!([first]), "chat"), stream_chunk(json!([second]), "chat")
            ], "done":true},
            "state":{"mode":"split", "providerCalls":1, "modelCalls":1},
            "observations":stream_commit_observations(1, 1)
        }),
    )
    .await;
    assert!(report.passed(), "{:?}", report.mismatches);
}

#[tokio::test(flavor = "current_thread")]
async fn stream_commit_validates_the_provider_when_it_has_the_final_hook() {
    let choice = stream_choice(0, "hello", Some("stop"));
    let event = stream_chunk(json!([choice]), "upstream");
    for (mode, expected) in [
        (
            "provider-framing",
            json!({
                "response":{"status":500},
                "state":{"mode":"provider-framing"},
                "error":{"code":"hook_failed", "status":500, "phase":"onStreamEvent"},
                "observations":stream_commit_observations(0, 0)
            }),
        ),
        (
            "provider-normal",
            json!({
                "response":{"status":200, "events":[stream_chunk(json!([choice]), "provider")],
                    "done":true},
                "state":{"mode":"provider-normal", "providerCalls":1},
                "observations":stream_commit_observations(1, 0)
            }),
        ),
    ] {
        let report = stream_commit_case(mode, std::slice::from_ref(&event), expected).await;
        assert!(report.passed(), "{mode}: {:?}", report.mismatches);
    }
}
