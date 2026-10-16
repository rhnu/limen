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
