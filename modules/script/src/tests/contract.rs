use std::time::{Duration, Instant};

use serde_json::json;

use crate::{
    Cancellation, HookCall, ObservationBatch, RuntimeOptions, ScriptErrorKind, ScriptRuntime,
    contract::*,
    tests::{Fixture, TestResult, ensure, hook, stop},
};

fn inputs() -> Vec<HookInput> {
    let head = || ResponseHead {
        status: 200,
        headers: Vec::new(),
    };
    let event = || EventInput {
        event: json!({"extension":{"custom":[1,null,true]}}),
        sse: Some(SseContext {
            event: "message".into(),
            id: Some("id".into()),
            retry: None,
        }),
    };
    let observer = || ObserverInput {
        outcome: HookOutcome::Failed,
        error: Some(json!({"code":"synthetic"})),
        stats: json!({"outputBytes":0}),
    };
    vec![
        HookInput::OnRequest {
            body: json!({"extension":{"custom":[1,null,true]}}),
        },
        HookInput::BeforeSend(RequestDraft {
            body: json!({}),
            url: "https://fictional.invalid".into(),
            headers: Vec::new(),
        }),
        HookInput::AfterReceive(head()),
        HookInput::OnResponse { body: json!({}) },
        HookInput::OnContent(event()),
        HookInput::OnUpstreamEvent(event()),
        HookInput::OnStreamEvent(event()),
        HookInput::OnErrorResponse(ErrorResponseInput {
            error: json!({"code":"synthetic"}),
            status: 502,
            headers: Vec::new(),
            body: Some("error".into()),
            content_type: None,
        }),
        HookInput::OnError(observer()),
        HookInput::OnFinish(observer()),
    ]
}

fn call(hook_id: u64, input: HookInput, state: ScriptState) -> HookCall {
    HookCall {
        call_id: 1,
        hook_id,
        context: HookContext {
            meta: HookMeta {
                request_id: "synthetic".into(),
                ..HookMeta::default()
            },
            client: ClientContext::default(),
            state,
            input,
        },
        deadline: Instant::now() + Duration::from_secs(2),
        cancellation: Cancellation::new(),
    }
}

#[tokio::test]
async fn typed_invocations_preserve_phase_effects_and_open_protocol_objects() -> TestResult {
    let fixture = Fixture::new()?;
    let path = fixture.write(
        "gateway.js",
        include_str!("../../fixtures/contract/phases.js"),
    )?;
    let (runtime, config) = ScriptRuntime::start(
        path,
        RuntimeOptions {
            observation_names: vec!["accepted".into()],
            ..RuntimeOptions::default()
        },
    )
    .await?;
    let id = hook(&config, "onRequest")?;
    for input in inputs() {
        let phase = input.phase();
        let output = runtime
            .invoke(call(id, input, ScriptState::new()))
            .await
            .outcome?;
        ensure(
            output.effects.observations.events == [0],
            "typed call lost approved observations",
        )?;
        match (phase, output.candidate) {
            (HookPhase::OnRequest, HookCandidate::OnRequest { body, rejection }) => ensure(
                body["extension"]["custom"] == json!([1, null, true, "kept"])
                    && rejection == Some(RejectionStatus::Forbidden),
                "request output lost open body or rejection",
            )?,
            (HookPhase::BeforeSend, HookCandidate::BeforeSend { request, rejection }) => ensure(
                request.url.ends_with("/changed")
                    && request.headers
                        == [HeaderField {
                            name: "x-owned".into(),
                            value: "value".into(),
                        }]
                    && rejection == Some(RejectionStatus::UnprocessableEntity),
                "request candidate lost controlled fields",
            )?,
            (HookPhase::AfterReceive, HookCandidate::AfterReceive(head)) => {
                ensure(head.status == 201, "response status edit was lost")?;
            },
            (HookPhase::OnResponse, HookCandidate::OnResponse { body }) => {
                ensure(body == json!({}), "body object was changed")?;
            },
            (HookPhase::OnContent, HookCandidate::OnContent { events })
            | (HookPhase::OnUpstreamEvent, HookCandidate::OnUpstreamEvent { events })
            | (HookPhase::OnStreamEvent, HookCandidate::OnStreamEvent { events }) => ensure(
                events
                    == [
                        json!({"extension":{"custom":[1,null,true]}}),
                        json!({"extra":"open"}),
                    ],
                "event candidate lost open extensions or order",
            )?,
            (HookPhase::OnErrorResponse, HookCandidate::OnErrorResponse(draft)) => ensure(
                draft.status == 503
                    && draft.body.is_none()
                    && draft.content_type.as_deref() == Some("text/plain"),
                "error response candidate lost nullable fields",
            )?,
            (HookPhase::OnError, HookCandidate::OnError)
            | (HookPhase::OnFinish, HookCandidate::OnFinish) => ensure(
                output.effects.state.is_empty(),
                "observer changed frozen state",
            )?,
            _ => return Err("candidate stage did not match the actual invocation".into()),
        }
        if !matches!(phase, HookPhase::OnError | HookPhase::OnFinish) {
            ensure(
                output.effects.state.get("committed") == Some(&json!(true)),
                "typed state was not committed",
            )?;
        }
    }
    stop(&runtime).await
}

#[tokio::test]
async fn public_input_and_output_shape_failures_remain_contract_errors() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start("export default () => ({onRequest(ctx) {if(ctx.state.invalid) ctx.body=[];}});")
        .await?;
    let id = hook(&config, "onRequest")?;
    let report = runtime
        .invoke(call(
            id,
            HookInput::OnRequest { body: json!([]) },
            ScriptState::new(),
        ))
        .await;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Contract)
            && report.timings.initial_call.is_none(),
        "invalid input entered a hook",
    )?;
    let report = runtime
        .invoke(call(
            id,
            HookInput::OnRequest { body: json!({}) },
            json!({"invalid":true}).as_object().unwrap().clone(),
        ))
        .await;
    ensure(
        report
            .outcome
            .is_err_and(|error| error.kind() == ScriptErrorKind::Contract)
            && report.timings.initial_call.is_some(),
        "invalid output was committed",
    )?;
    stop(&runtime).await
}

#[test]
fn candidate_decode_uses_actual_phase_and_rejects_malformed_closed_fields() {
    for (phase, context) in [
        (
            HookPhase::BeforeSend,
            json!({"state":{},"body":{},"url":false,"headers":[]}),
        ),
        (
            HookPhase::AfterReceive,
            json!({"state":{},"status":"200","headers":[]}),
        ),
        (HookPhase::OnResponse, json!({"state":{},"body":[]})),
        (HookPhase::OnContent, json!({"state":{},"events":{}})),
        (
            HookPhase::OnErrorResponse,
            json!({"state":{},"status":502,"headers":[],"body":3,"contentType":null}),
        ),
        (
            HookPhase::OnFinish,
            json!({"state":{},"rejection":{"status":400}}),
        ),
        (
            HookPhase::OnRequest,
            json!({"state":{},"body":{},"rejection":{"status":401}}),
        ),
        (HookPhase::OnRequest, json!({"state":[],"body":{}})),
    ] {
        assert_eq!(
            decode_candidate(context, phase, ObservationBatch::default())
                .unwrap_err()
                .kind(),
            ScriptErrorKind::Contract
        );
    }
    assert!(matches!(
        decode_candidate(
            json!({"meta":{"phase":"onRequest"},"state":{}}),
            HookPhase::OnFinish,
            ObservationBatch::default()
        )
        .unwrap()
        .candidate,
        HookCandidate::OnFinish
    ));
}

#[test]
fn state_and_rejection_decoding_reject_null_wrong_types_and_extra_fields() {
    for state in [json!(null), json!([]), json!(false), json!("state")] {
        assert_eq!(
            decode_candidate(
                json!({"state":state,"body":{}}),
                HookPhase::OnRequest,
                ObservationBatch::default()
            )
            .unwrap_err()
            .kind(),
            ScriptErrorKind::Contract
        );
    }
    for rejection in [
        json!(null),
        json!([]),
        json!(400),
        json!({}),
        json!({"status":null}),
        json!({"status":"400"}),
        json!({"status":400.5}),
        json!({"status":-1}),
        json!({"status":500}),
        json!({"status":400,"extra":true}),
    ] {
        assert_eq!(
            decode_candidate(
                json!({"state":{},"body":{},"rejection":rejection}),
                HookPhase::OnRequest,
                ObservationBatch::default()
            )
            .unwrap_err()
            .kind(),
            ScriptErrorKind::Contract
        );
    }
    for (status, expected) in [
        (400, RejectionStatus::BadRequest),
        (403, RejectionStatus::Forbidden),
        (422, RejectionStatus::UnprocessableEntity),
    ] {
        let output = decode_candidate(
            json!({"state":{},"body":{},"rejection":{"status":status}}),
            HookPhase::OnRequest,
            ObservationBatch::default(),
        )
        .unwrap();
        assert!(
            matches!(output.candidate, HookCandidate::OnRequest { rejection:Some(actual), .. } if actual == expected)
        );
    }
}
