use std::time::Duration;

use limen_config::ConfigErrorKind;
use serde_json::json;

use super::*;

fn config(policy: &serde_json::Value) -> BusinessConfig {
    BusinessConfig::from_value(json!({
        "providers":{"p":{"endpoint":"https://synthetic.invalid/chat","pipeline":[{"name":"native","builtin":policy}]}},
        "models":{"chat":{"provider":"p","upstream":"model"}}
    }), &ResourceLimits::default()).unwrap()
}

#[test]
fn compilation_obeys_startup_deadline_and_native_memory_budget() {
    let config = config(
        &json!({"kind":"replaceText","search":"abc","replacement":"x","maxBufferedChars":2}),
    );
    assert!(matches!(
        ExecutionPlans::compile(&config, &ResourceLimits::default(), &[], Instant::now()),
        Err(Error::Deadline)
    ));
    let limits = ResourceLimits {
        max_output_bytes: 1,
        ..ResourceLimits::default()
    };
    assert!(matches!(
        ExecutionPlans::compile(
            &config,
            &limits,
            &[],
            Instant::now() + Duration::from_secs(1)
        ),
        Err(Error::Content(NativeContentError::Limit))
    ));
}

#[test]
fn observations_are_bound_at_startup_and_keep_registered_ids() {
    let config = config(&json!({"kind":"capCompletionTokens","max":10,"onChange":"changed"}));
    let deadline = Instant::now() + Duration::from_secs(1);
    assert!(
        matches!(ExecutionPlans::compile(&config, &ResourceLimits::default(), &[], deadline), Err(Error::Config(error)) if error.kind == ConfigErrorKind::Reference)
    );
    let plans = ExecutionPlans::compile(
        &config,
        &ResourceLimits::default(),
        &["other".into(), "changed".into()],
        deadline,
    )
    .unwrap();
    let route = plans.route(&config, "chat").unwrap();
    let (provider, _) = route.before_send();
    let BeforeSendExecutor::Native(policy) = &provider[0].executor else {
        panic!("expected native");
    };
    let mut request = limen_script::RequestDraft {
        body: json!({}),
        url: "https://synthetic.invalid/chat".into(),
        headers: Vec::new(),
    };
    let (_, observations) = crate::gateway::native::before_send(
        policy,
        &mut request,
        &limen_script::ClientContext::default(),
        "native",
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(request.body["max_completion_tokens"], 10);
    assert_eq!(observations.events, vec![1]);
}
