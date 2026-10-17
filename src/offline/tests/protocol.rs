use std::fmt::Write as _;

use limen_config::{ResourceLimits, load_server};

use super::{Report, Value, execute, fs, json, run_case};
use crate::bootstrap::Loaded;

enum Input<'a> {
    Sse(&'a str),
    Json(Value),
    RawJson(&'a str),
}

async fn run(
    input: Input<'_>,
    stream: bool,
    steps: [&str; 2],
    expect: Value,
    limits: Option<ResourceLimits>,
) -> Report {
    let directory = tempfile::tempdir().unwrap();
    let server = directory.path().join("server.toml");
    fs::write(
        &server,
        "[auth]\napi_key = \"synthetic-client-secret-0123456789\"\n",
    )
    .unwrap();
    fs::write(directory.path().join("config.js"), format!(
        "export default {{ providers: {{ p: {{ endpoint: 'https://upstream.invalid/chat', pipeline: [{}] }} }}, models: {{ chat: {{ provider: 'p', upstream: 'real-model', pipeline: [{}] }} }} }};",
        steps[0], steps[1],
    )).unwrap();
    let upstream = match input {
        Input::Sse(bytes) => {
            fs::write(directory.path().join("input.sse"), bytes).unwrap();
            json!({"status":200,"headers":{"content-type":["text/event-stream"]},"bodyFile":"input.sse"})
        },
        Input::Json(body) => {
            json!({"status":200,"headers":{"content-type":["application/json"]},"body":body})
        },
        Input::RawJson(bytes) => {
            fs::write(directory.path().join("input.json"), bytes).unwrap();
            json!({"status":200,"headers":{"content-type":["application/json"]},"bodyFile":"input.json"})
        },
    };
    let case = json!({"name":"protocol-boundary",
        "request":{"model":"chat","messages":[{"role":"user","content":"synthetic"}],"stream":stream},
        "upstream":upstream,"expect":expect});
    let path = directory.path().join("input.case.json");
    fs::write(&path, serde_json::to_vec(&case).unwrap()).unwrap();
    if let Some(limits) = limits {
        let (path, settings) = load_server(Some(&server)).unwrap();
        let mut loaded = Loaded::start(&path, settings).await.unwrap();
        loaded.server.limits = limits;
        let report = run_case(&loaded, &directory.path().join("input.case.json")).await;
        assert!(loaded.shutdown().await.is_ok());
        report.unwrap()
    } else {
        execute(&server, &path).await.unwrap()
    }
}

fn assert_passed(report: &Report) {
    assert!(report.passed(), "{}", report.mismatches.join(", "));
}

fn chunk(delta: Value, finish: Value) -> Value {
    let mut event = json!({"id":"protocol","object":"chat.completion.chunk","created":1,"model":"real-model",
        "choices":[{"index":0}]});
    event["choices"][0]["delta"] = delta;
    event["choices"][0]["finish_reason"] = finish;
    event
}

fn encoded(events: &[Value], done: bool) -> String {
    let mut bytes = String::new();
    for event in events {
        writeln!(bytes, "data: {event}\n").unwrap();
    }
    if done {
        bytes.push_str("data: [DONE]\n\n");
    }
    bytes
}

fn aliased(events: &[Value]) -> Vec<Value> {
    events
        .iter()
        .cloned()
        .map(|mut event| {
            if event.get("model").is_some() {
                event["model"] = json!("chat");
            }
            event
        })
        .collect()
}

fn usage(source: &str, completed: bool) -> Value {
    json!({"inputTokens":null,"outputTokens":null,"totalTokens":null,
        "cachedInputTokens":null,"reasoningOutputTokens":null,"quality":"missing",
        "source":source,"upstreamCompleted":completed})
}

fn error(code: &str, phase: &str) -> Value {
    let status = if code == "hook_failed" { 500 } else { 502 };
    json!({"code":code,"status":status,"phase":phase})
}

#[tokio::test(flavor = "current_thread")]
async fn maximum_json_depth_is_supported_by_both_upstream_formats() {
    let extension = format!("{}null{}", "[".repeat(127), "]".repeat(127));
    let json_body = format!("{{\"extension\":{extension}}}");
    let sse = format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"extension\":{extension}}}\n\ndata: [DONE]\n\n"
    );
    for (input, stream, source) in [
        (Input::RawJson(&json_body), false, "json"),
        (Input::Sse(&sse), true, "sse"),
    ] {
        let mut response = json!({"status":200});
        if stream {
            response["done"] = json!(true);
        }
        assert_passed(
            &run(
                input,
                stream,
                ["", ""],
                json!({"response":response,"stats":{"usage":usage(source,true)}}),
                Some(ResourceLimits {
                    max_json_depth: 128,
                    ..ResourceLimits::default()
                }),
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn named_frame_metadata_is_frozen_and_provider_model_hooks_stay_ordered() {
    let event = chunk(json!({"role":"assistant","content":"hello"}), json!("stop"));
    let bytes = format!(
        ": heartbeat\n\nevent: update\nid: frame-1\nretry: 1500\ndata: {event}\n\ndata: [DONE]\n\n"
    );
    let provider = r"{ name: 'provider',
        onUpstreamEvent(ctx) {
            if (!Object.isFrozen(ctx.sse)) throw new Error('metadata not frozen');
            let rejected = false;
            try { ctx.sse.id = 'changed'; } catch { rejected = true; }
            if (!rejected) throw new Error('metadata writable');
            ctx.state.order = ['provider-upstream'];
            ctx.state.metadata = ctx.sse;
        },
        onStreamEvent(ctx) {
            ctx.state.order.push('provider-stream');
            ctx.state.streamMetadata = ctx.sse;
        }
    }";
    let model = r"{ name: 'model',
        onUpstreamEvent(ctx) { ctx.state.order.push('model-upstream'); },
        onStreamEvent(ctx) { ctx.state.order.push('model-stream'); }
    }";
    let metadata = json!({"event":"update","id":"frame-1","retry":"1500"});
    let expect = json!({"sent":true,"response":{"status":200,"events":aliased(&[event]),"done":true},
        "state":{"order":["provider-upstream","model-upstream","provider-stream","model-stream"],
            "metadata":metadata,"streamMetadata":metadata},
        "stats":{"usage":usage("sse",true)}});
    assert_passed(&run(Input::Sse(&bytes), true, [provider, model], expect, None).await);
}

fn semantic_events() -> Vec<Value> {
    vec![
        chunk(
            json!({"role":"assistant","content":[{"type":"text","text":"hello"}],
            "reasoning_content":"thinking","reasoning_details":[{"type":"reasoning.text","index":0,"text":"thought"}],
            "audio":{"id":"audio-1","data":"YQ==","transcript":"spoken"}}),
            Value::Null,
        ),
        chunk(json!({}), json!("stop")),
        json!({"id":"protocol","object":"chat.completion.chunk","created":1,"model":"real-model",
            "choices":[{"index":0,"delta":{"audio":{"expires_at":100}},
                "annotations":[{"type":"citation","url":"https://example.invalid/"}],"content_filter_results":{}}]}),
        json!({"choices":[{"index":0,"content_filter_results":{}}]}),
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn auxiliary_events_content_arrays_reasoning_and_audio_survive_same_shape_and_aggregation() {
    let events = semantic_events();
    let bytes = encoded(&events, true);
    let message = json!({"role":"assistant","content":[{"type":"text","text":"hello"}],
        "reasoning_content":"thinking","reasoning_details":[{"type":"reasoning.text","index":0,"text":"thought"}],
        "audio":{"id":"audio-1","data":"YQ==","transcript":"spoken","expires_at":100}});
    for stream in [false, true] {
        let response = if stream {
            json!({"status":200,"events":aliased(&events),"done":true})
        } else {
            json!({"status":200,"body":{"id":"protocol","object":"chat.completion","created":1,"model":"chat",
                "choices":[{"index":0,"message":message,"finish_reason":"stop",
                    "annotations":[{"type":"citation","url":"https://example.invalid/"}],"content_filter_results":{}}]}})
        };
        assert_passed(
            &run(
                Input::Sse(&bytes),
                stream,
                ["", ""],
                json!({"response":response,"stats":{"usage":usage("sse",true)}}),
                None,
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn json_conversion_exposes_all_semantic_fields_to_stream_hooks() {
    let message = json!({"role":"assistant","content":[{"type":"text","text":"hello"}],
        "reasoning_content":"thinking","reasoning_details":[{"type":"reasoning.text","index":0,"text":"thought"}],
        "audio":{"id":"audio-1","data":"YQ==","transcript":"spoken","expires_at":100}});
    let annotations = json!([{"type":"citation","url":"https://example.invalid/"}]);
    let logprobs =
        json!({"content":[{"token":"hello","logprob":-0.1,"bytes":[104],"top_logprobs":[]}]});
    let body = json!({"id":"protocol","object":"chat.completion","created":1,"model":"real-model",
        "choices":[{"index":0,"message":message,"annotations":annotations,"logprobs":logprobs,"finish_reason":"stop"}]});
    let inspect = r"{ name: 'inspect', onStreamEvent(ctx) {
        if (ctx.sse !== null) throw new Error('synthetic transport metadata');
        const choice = ctx.event.choices[0];
        if (!choice) return;
        ctx.state.message ||= {};
        Object.assign(ctx.state.message, choice.delta || {});
        if ('annotations' in choice) ctx.state.annotations = choice.annotations;
        if ('logprobs' in choice) ctx.state.logprobs = choice.logprobs;
        if (choice.finish_reason) ctx.state.finish = choice.finish_reason;
    } }";
    assert_passed(&run(Input::Json(body), true, [inspect,""],
        json!({"response":{"status":200,"done":true},
            "state":{"message":message,"annotations":annotations,"logprobs":logprobs,"finish":"stop"},
            "stats":{"usage":usage("json",true)}}), None).await);
}

#[tokio::test(flavor = "current_thread")]
async fn done_never_masks_unfinished_choices_or_an_unterminated_frame() {
    let unfinished = chunk(json!({"role":"assistant","content":"hello"}), Value::Null);
    let complete = chunk(json!({"role":"assistant","content":"hello"}), json!("stop"));
    let residual = format!("{}data: {{\"choices\":", encoded(&[complete], false));
    for bytes in [encoded(&[unfinished], true), residual] {
        assert_passed(
            &run(
                Input::Sse(&bytes),
                false,
                ["", ""],
                json!({"response":{"status":502},"error":error("upstream_protocol","upstream_sse"),
                "stats":{"usage":usage("sse",false)}}),
                None,
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn extensions_pass_through_but_unknown_cross_shape_semantics_fail() {
    let mut event = chunk(json!({"role":"assistant","content":"hello"}), json!("stop"));
    event["vendor_extension"] = json!({"opaque":[1,2]});
    let bytes = encoded(&[event.clone()], true);
    assert_passed(
        &run(
            Input::Sse(&bytes),
            true,
            ["", ""],
            json!({"response":{"status":200,"events":aliased(&[event]),"done":true}}),
            None,
        )
        .await,
    );
    assert_passed(
        &run(
            Input::Sse(&bytes),
            false,
            ["", ""],
            json!({"response":{"status":502},"error":error("upstream_protocol","sse_to_json"),
            "stats":{"usage":usage("sse",false)}}),
            None,
        )
        .await,
    );

    let body = json!({"id":"protocol","object":"chat.completion","created":1,"model":"real-model",
        "choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],
        "vendor_extension":{"opaque":[1,2]}});
    assert_passed(
        &run(
            Input::Json(body),
            true,
            ["", ""],
            json!({"response":{"status":502},"error":error("upstream_protocol","json_to_sse"),
            "stats":{"usage":usage("json",true)}}),
            None,
        )
        .await,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn downstream_drop_cannot_erase_native_protocol_completion() {
    let bytes = encoded(
        &[chunk(
            json!({"role":"assistant","content":"hello"}),
            json!("stop"),
        )],
        true,
    );
    let drop = "{name:'drop',onUpstreamEvent(ctx){ctx.replaceEvents([]);}}";
    assert_passed(
        &run(
            Input::Sse(&bytes),
            true,
            [drop, ""],
            json!({"response":{"status":500},"error":error("hook_failed","onStreamEvent"),
            "stats":{"usage":usage("sse",true)}}),
            None,
        )
        .await,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn comment_heartbeat_commits_stream_status_before_a_native_error() {
    let native = json!({"error":{"code":"synthetic_error","message":"synthetic detail"}});
    for comment in [false, true] {
        let bytes = format!(
            "{}event: error\ndata: {native}\n\n",
            if comment { ": heartbeat\n\n" } else { "" }
        );
        let response = if comment {
            json!({"status":200,"events":[],"done":false})
        } else {
            json!({"status":502,"body":native})
        };
        assert_passed(
            &run(
                Input::Sse(&bytes),
                true,
                ["", ""],
                json!({"response":response,"error":error("upstream_protocol","upstream_sse"),
                "stats":{"usage":usage("sse",false)}}),
                None,
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_frames_and_native_json_keep_independent_resource_limits() {
    let complete = chunk(json!({"role":"assistant","content":"hello"}), json!("stop"));
    let large_frame = format!(
        "id: {}\ndata: {complete}\n\ndata: [DONE]\n\n",
        "x".repeat(400)
    );
    let mut many_nodes = complete.clone();
    many_nodes["vendor_extension"] = json!(vec![0; 150]);
    let mut deep = complete.clone();
    let mut nested = json!(0);
    for _ in 0..20 {
        nested = json!({"nested":nested});
    }
    deep["vendor_extension"] = nested;
    let mut unsafe_number = complete;
    unsafe_number["vendor_extension"] = json!(9_007_199_254_740_992_u64);
    for (bytes, limits) in [
        (
            large_frame,
            ResourceLimits {
                max_event_bytes: 300,
                ..ResourceLimits::default()
            },
        ),
        (
            encoded(&[many_nodes], true),
            ResourceLimits {
                max_json_nodes: 100,
                ..ResourceLimits::default()
            },
        ),
        (
            encoded(&[deep], true),
            ResourceLimits {
                max_json_depth: 12,
                ..ResourceLimits::default()
            },
        ),
        (encoded(&[unsafe_number], true), ResourceLimits::default()),
    ] {
        assert_passed(
            &run(
                Input::Sse(&bytes),
                true,
                ["", ""],
                json!({"response":{"status":502},"error":error("upstream_protocol","upstream_sse"),
                "stats":{"usage":usage("sse",false)}}),
                Some(limits),
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unchanged_native_sse_uses_raw_frame_size_instead_of_normalized_json_size() {
    // The complete frame is 161 bytes; serializing its parsed data would take
    // 333 bytes because the scientific-notation extensions expand.
    let numbers = ["1e9"; 20].join(",");
    let bytes = format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"extension\":[{numbers}]}}\n\ndata: [DONE]\n\n"
    );
    assert_passed(
        &run(
            Input::Sse(&bytes),
            true,
            ["", ""],
            json!({"sent":true,"response":{"status":200,"done":true,
                "events":[{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                    "extension":vec![1_000_000_000.0;20]}]},
                "stats":{"usage":usage("sse",true)}}),
            Some(ResourceLimits {
                max_event_bytes: 200,
                ..ResourceLimits::default()
            }),
        )
        .await,
    );
}

#[tokio::test(flavor = "current_thread")]
async fn alias_and_hook_rewrites_still_enforce_actual_encoded_frame_limits() {
    let numbers = ["1e9"; 8].join(",");
    let bytes = format!(
        "id: {}\ndata: {{\"model\":\"x\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"extension\":[{numbers}]}}\n\ndata: [DONE]\n\n",
        "i".repeat(60)
    );
    // Both the source frame and each rewritten JSON payload fit 200 bytes.
    // Re-encoding data while preserving the ID makes the actual frame too large.
    for step in [
        "",
        "{name:'rewrite',onStreamEvent(ctx){ctx.event.extension[0]=2000000000;ctx.state.attempted=true;}}",
    ] {
        assert_passed(
            &run(
                Input::Sse(&bytes),
                true,
                [step, ""],
                json!({"response":{"status":500},"state":{},
                    "error":error("hook_failed","onStreamEvent")}),
                Some(ResourceLimits {
                    max_event_bytes: 200,
                    ..ResourceLimits::default()
                }),
            )
            .await,
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn provider_and_model_share_one_bounded_event_expansion_budget() {
    let bytes = encoded(
        &[chunk(
            json!({"role":"assistant","content":"hello"}),
            json!("stop"),
        )],
        true,
    );
    let provider = "{name:'provider',onUpstreamEvent(ctx){ctx.state.provider=true;ctx.replaceEvents([ctx.event,ctx.event]);}}";
    let model = "{name:'model',onUpstreamEvent(ctx){ctx.state.model=true;ctx.replaceEvents([ctx.event,ctx.event]);}}";
    assert_passed(
        &run(
            Input::Sse(&bytes),
            true,
            [provider, model],
            json!({"response":{"status":500},"error":error("hook_failed","onUpstreamEvent"),
            "state":{"provider":true},"stats":{"usage":usage("sse",false)}}),
            Some(ResourceLimits {
                max_output_events: 3,
                ..ResourceLimits::default()
            }),
        )
        .await,
    );
}
