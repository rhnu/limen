use std::{fmt::Write as _, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Request},
};
use limen_chat::{Aggregator, Frame, FrameDecoder};
use limen_script::Cancellation;
use serde_json::{Value, json};

use super::{fs, group};
use crate::{bootstrap::Loaded, gateway::FixtureResponse};

mod ordered;

struct Outcome {
    status: u16,
    body: Value,
    done: bool,
    state: Value,
    error: Option<Value>,
    stats: Value,
    outbound: Value,
    observations: Value,
    events: Vec<Value>,
    frames: Vec<Frame>,
}

async fn run(config: &str, source: Value, chunks: Option<&str>, stream: bool) -> Outcome {
    let directory =
        tempfile::tempdir_in(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
            .unwrap();
    let server = directory.path().join("server.toml");
    fs::write(
        &server,
        "[auth]\napi_key = \"synthetic-client-secret-0123456789\"\n[limits]\nexecution_ms=1000\n[observability]\nevents=['semantic']\n",
    )
    .unwrap();
    fs::write(directory.path().join("gateway.js"), config).unwrap();
    let (path, settings) = Loaded::settings(Some(&server)).unwrap();
    let loaded = Loaded::start(&path, settings).await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        HeaderValue::from_static(if chunks.is_some() {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    let bytes = chunks.map_or_else(
        || serde_json::to_vec(&source).unwrap(),
        |text| text.as_bytes().to_vec(),
    );
    let (gateway, trace) = crate::offline::application(
        &loaded,
        FixtureResponse {
            status: 200,
            headers,
            body: bytes,
        },
    );
    let request = Request::builder().method("POST").uri("/v1/chat/completions")
        .header("authorization", "Bearer synthetic-client-secret-0123456789")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model":"chat","messages":[{"role":"user","content":"public prompt"}],"stream":stream}).to_string())).unwrap();
    let response = Arc::clone(&gateway)
        .handle(request, Cancellation::new())
        .await;
    let status = response.status().as_u16();
    let sse = response
        .headers()
        .get("content-type")
        .is_some_and(|v| v == "text/event-stream");
    let bytes = to_bytes(response.into_body(), loaded.server.limits.max_output_bytes)
        .await
        .unwrap();
    gateway.drain().await;
    let (body, done, events, frames) = if sse {
        let (events, done) = super::super::stream(&bytes, &loaded.server.limits).unwrap();
        let body = if done {
            aggregate_events(&events, &loaded.server.limits)
        } else {
            Value::Null
        };
        let mut parser = FrameDecoder::new(loaded.server.limits.max_event_bytes.max(1024));
        let frames = bytes
            .iter()
            .filter_map(|byte| parser.push(*byte).unwrap())
            .collect();
        (body, done, events, frames)
    } else {
        (
            serde_json::from_slice(&bytes).unwrap(),
            false,
            Vec::new(),
            Vec::new(),
        )
    };
    let trace = trace.lock().await;
    let result = Outcome {
        status,
        body,
        done,
        state: trace.state.clone(),
        error: trace.error.clone(),
        stats: trace.stats.clone(),
        outbound: trace
            .sent
            .as_ref()
            .map_or(Value::Null, |sent| sent.body.clone()),
        observations: trace.observations.clone(),
        events,
        frames,
    };
    assert!(loaded.shutdown().await.is_ok());
    result
}

fn aggregate_events(events: &[Value], limits: &limen_config::ResourceLimits) -> Value {
    let mut aggregate = Aggregator::new(limits.max_aggregate_bytes, limits.max_aggregate_events);
    for event in events {
        aggregate
            .push(event.clone())
            .unwrap_or_else(|error| panic!("{error:?}: {event}"));
    }
    aggregate
        .finish()
        .unwrap_or_else(|error| panic!("{error:?}: {events:?}"))
}

fn config(imports: &str, provider: &str, model: &str, extra: &str) -> String {
    format!(
        "import {{defineGateway}} from 'limen:gateway'; {imports}\nexport default defineGateway(gateway=>{{ const p=gateway.provider('p',{{endpoint:'https://upstream.invalid/chat'}}); for (const step of [{provider}]) p.use(step); const m=p.model('chat',{{upstream:'real-model'}}); for (const step of [{model}]) m.use(step); {extra} }});"
    )
}

fn complete(message: Value, finish: &str) -> Value {
    let mut response = json!({"id":"semantic","object":"chat.completion","created":1,"model":"real-model",
        "choices":[{"index":0,"finish_reason":finish}],"usage":{"total_tokens":10}});
    response["choices"][0]["message"] = message;
    response
}

fn chunk(delta: Value, finish: Option<&str>) -> Value {
    let mut event = json!({"id":"semantic","object":"chat.completion.chunk","created":1,"model":"real-model",
        "choices":[{"index":0,"finish_reason":finish}]});
    event["choices"][0]["delta"] = delta;
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

#[tokio::test(flavor = "current_thread")]
async fn native_and_script_steps_share_provider_model_order() {
    let settings = r"
        import {defineGateway} from 'limen:gateway';
        import {replaceText} from 'limen:content';
        export default defineGateway(gateway => {
            const provider = gateway.provider('p', {endpoint:'https://upstream.invalid/chat'});
            provider.use(replaceText({name:'first',search:'old',replacement:'middle',maxBufferedChars:2}));
            provider.step('inspect-provider', step => {
                step.beforeSend(ctx => { ctx.state.order = ['provider']; });
                step.onContent(ctx => {
                    if(ctx.event.type==='part_delta') ctx.state.seen=(ctx.state.seen||'')+ctx.event.text;
                });
            });
            const model = provider.model('chat', {upstream:'real-model'});
            model.step('inspect-model', step => {
                step.beforeSend(ctx => { ctx.state.order.push('model'); });
            });
            model.use(replaceText({name:'last',search:'middle',replacement:'new',maxBufferedChars:5}));
        });
    ";
    let input = encoded(
        &[
            chunk(json!({"role":"assistant","content":"ol"}), None),
            chunk(json!({"content":"d"}), None),
            chunk(json!({}), Some("stop")),
        ],
        true,
    );
    for stream in [false, true] {
        for sse in [false, true] {
            let result = run(
                settings,
                complete(json!({"role":"assistant","content":"old"}), "stop"),
                sse.then_some(input.as_str()),
                stream,
            )
            .await;
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.body["choices"][0]["message"]["content"], "new");
            assert_eq!(
                result.state,
                json!({"order":["provider","model"],"seen":"middle"})
            );
            assert!(
                result.stats["timings"]["native"]["onContent"]["calls"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn common_text_transform_covers_four_paths_and_every_delta_split() {
    let settings = config(
        "import {replaceText} from 'limen:content';",
        "replaceText({name:'literal',search:'aba',replacement:'Z',maxBufferedChars:2})",
        "",
        "",
    );
    let source = "xxabaxx";
    for stream in [false, true] {
        let result = run(
            &settings,
            complete(json!({"role":"assistant","content":source}), "stop"),
            None,
            stream,
        )
        .await;
        assert_eq!(result.status, 200);
        assert_eq!(result.body["choices"][0]["message"]["content"], "xxZxx");
        assert_eq!(result.state, json!({}));
        assert_eq!(result.done, stream);
        for split in 0..=source.len() {
            let input = encoded(
                &[
                    chunk(json!({"role":"assistant","content":&source[..split]}), None),
                    chunk(json!({"content":&source[split..]}), None),
                    chunk(json!({}), Some("stop")),
                ],
                true,
            );
            let result = run(&settings, Value::Null, Some(&input), stream).await;
            assert_eq!(result.status, 200);
            assert!(result.error.is_none(), "split {split}: {:?}", result.error);
            assert_eq!(
                result.body["choices"][0]["message"]["content"], "xxZxx",
                "split {split}"
            );
            assert_eq!(result.state, json!({}));
            assert_eq!(result.done, stream);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn semantic_edits_preserve_open_json_extensions_and_opaque_parts() {
    let settings = config(
        "",
        "{name:'edit',onContent(ctx){if(ctx.event.type==='part_delta')ctx.event.text=ctx.event.text.toUpperCase();}}",
        "",
        "",
    );
    let mut input = complete(
        json!({"role":"assistant","content":[{"type":"text","text":"hello","extra":7},
        {"type":"image_url","image_url":{"url":"public:image"}}],"message_extension":{"keep":true}}),
        "stop",
    );
    input["extension"] = json!({"opaque":[1,2,3],"annotations":{"vendor":"opaque"}});
    input["choices"][0]["choice_extension"] = json!(["keep"]);
    let mut expected = input.clone();
    expected["model"] = json!("chat");
    expected["choices"][0]["message"]["content"][0]["text"] = json!("HELLO");
    let result = run(&settings, input, None, false).await;
    assert_eq!(result.status, 200);
    assert_eq!(result.body, expected);
    let result = run(
        &settings,
        json!({"opaque":{"no_choices":true}}),
        None,
        false,
    )
    .await;
    assert_eq!(result.body, json!({"opaque":{"no_choices":true}}));
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_semantic_step_discards_its_entire_source_batch() {
    let settings = config(
        "",
        "{name:'accepted',onContent(ctx){ctx.state.previous=true;ctx.observe.record('semantic');}}",
        "{name:'invalid',onContent(ctx){ctx.state.attempted=true;ctx.observe.record('semantic');if(ctx.event.type==='part_delta')ctx.event.text=7;}}",
        "",
    );
    let result = run(
        &settings,
        complete(json!({"content":"hello"}), "stop"),
        None,
        false,
    )
    .await;
    assert_eq!(result.status, 500);
    assert_eq!(result.error.as_ref().unwrap()["phase"], "onContent");
    assert_eq!(result.state, json!({"previous":true}));
    assert_eq!(
        result.observations,
        json!({"events":[{"event":"semantic","phase":"onContent","step":"accepted","count":4}],"dropped":0})
    );
}

#[tokio::test(flavor = "current_thread")]
async fn failed_render_and_late_generation_cannot_publish_state_or_success() {
    let settings = config(
        "",
        "{name:'edit',beforeSend(ctx){ctx.state.before=true;},onContent(ctx){ctx.state.attempted=true;if(ctx.event.type==='part_delta')ctx.event.text+='!';}}",
        "",
        "",
    );
    let mut input = complete(json!({"content":"hello"}), "stop");
    input["choices"][0]["logprobs"] = json!({"content":[{"token":"hello","logprob":-1}]});
    let result = run(&settings, input, None, false).await;
    assert_eq!(result.status, 500);
    assert_eq!(result.state, json!({"before":true}));
    let settings = config(
        "",
        "{name:'late',onContent(ctx){if(ctx.event.type==='response_end'){ctx.state.late=true;ctx.replaceEvents([{type:'part_delta',choice:0,part:'text:0',text:'late'},ctx.event]);}}}",
        "",
        "",
    );
    let result = run(
        &settings,
        complete(json!({"content":"hello"}), "stop"),
        None,
        true,
    )
    .await;
    assert_eq!(result.status, 500);
    assert!(!result.done);
    assert_eq!(result.state, json!({}));
}

#[tokio::test(flavor = "current_thread")]
async fn source_truncation_never_runs_successful_semantic_end() {
    let settings = config(
        "",
        "{name:'end',onContent(ctx){if(ctx.event.type==='response_end')ctx.state.ended=true;}}",
        "",
        "",
    );
    let input = encoded(
        &[
            chunk(json!({"content":"prefix"}), None),
            chunk(json!({}), Some("stop")),
        ],
        false,
    );
    for stream in [false, true] {
        let result = run(&settings, Value::Null, Some(&input), stream).await;
        assert!(!result.done);
        assert_eq!(result.error.as_ref().unwrap()["code"], "upstream_protocol");
        assert_eq!(result.state, json!({}));
        assert_eq!(result.stats["usage"]["upstreamCompleted"], false);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn semantic_steps_keep_independent_output_lifecycles() {
    let settings = config(
        "",
        "{name:'source',onContent(ctx){if(ctx.event.type==='part_delta')ctx.state.input=(ctx.state.input||'')+ctx.event.text;}}",
        "{name:'remove',onContent(ctx){if(ctx.event.type.startsWith('part_'))ctx.replaceEvents([]);}}",
        "",
    );
    let bytes = encoded(
        &[
            chunk(json!({"role":"assistant","content":"hel"}), None),
            chunk(json!({"content":"lo"}), None),
            chunk(json!({}), Some("stop")),
        ],
        true,
    );
    for stream in [false, true] {
        let result = run(&settings, Value::Null, Some(&bytes), stream).await;
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(result.state, json!({"input":"hello"}));
        assert!(
            result.body["choices"][0]["message"]
                .get("content")
                .is_none()
        );
        assert_eq!(result.body["choices"][0]["finish_reason"], "stop");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn complete_part_mapping_is_common_to_four_paths() {
    let settings = config(
        "import {mapPartText} from 'limen:content';",
        "mapPartText({name:'complete',kinds:['text'],maxBufferedChars:32,transform:text=>text.replace(/a+b/g,'X')})",
        "",
        "",
    );
    let bytes = encoded(
        &[
            chunk(json!({"role":"assistant","content":"aa"}), None),
            chunk(json!({"content":"ab"}), None),
            chunk(json!({}), Some("stop")),
        ],
        true,
    );
    for stream in [false, true] {
        for sse in [false, true] {
            let result = run(
                &settings,
                complete(json!({"role":"assistant","content":"aaab"}), "stop"),
                sse.then_some(bytes.as_str()),
                stream,
            )
            .await;
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.body["choices"][0]["message"]["content"], "X");
            assert_eq!(result.state, json!({}));
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn runnable_content_examples_cover_all_four_transports() {
    group(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/content")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn synthetic_tool_is_unwrapped_in_all_paths_and_keeps_actual_tools() {
    let settings = config(
        "import {syntheticTextTool} from 'limen:content';",
        "{name:'source-view',onContent(ctx){ctx.state.observed=true;}},syntheticTextTool({name:'transport',maxBufferedChars:1024})",
        "",
        "",
    );
    let actual =
        json!({"id":"actual","type":"function","function":{"name":"lookup","arguments":"{}"}});
    let synthetic = json!({"id":"text","type":"function","function":{"name":"limen_emit_text","arguments":"{\"text\":\"hello \\uD83D\\uDE00\"}"}});
    for mixed in [false, true] {
        for stream in [false, true] {
            let calls = if mixed {
                vec![synthetic.clone(), actual.clone()]
            } else {
                vec![synthetic.clone()]
            };
            let result = run(
                &settings,
                complete(
                    json!({"role":"assistant","content":null,"tool_calls":calls}),
                    "tool_calls",
                ),
                None,
                stream,
            )
            .await;
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.body["choices"][0]["message"]["content"], "hello 😀");
            assert_eq!(
                result.body["choices"][0]["finish_reason"],
                if mixed { "tool_calls" } else { "stop" }
            );
            if mixed {
                assert_eq!(
                    result.body["choices"][0]["message"]["tool_calls"],
                    json!([actual.clone()])
                );
            } else {
                assert!(
                    result.body["choices"][0]["message"]
                        .get("tool_calls")
                        .is_none()
                );
            }
            assert_eq!(result.stats["usage"]["totalTokens"], 10);
            assert_eq!(
                result.outbound["messages"][0],
                json!({"role":"user","content":"public prompt"})
            );
            let mut events = vec![
                chunk(
                    json!({"role":"assistant","tool_calls":[{"index":0,"id":"text","type":"function","function":{"name":"limen_","arguments":""}}]}),
                    None,
                ),
                chunk(
                    json!({"tool_calls":[{"index":0,"function":{"name":"emit_text","arguments":"{\"text\":\"hello \\uD83D"}}]}),
                    None,
                ),
                chunk(
                    json!({"tool_calls":[{"index":0,"function":{"arguments":"\\uDE00\"}"}}]}),
                    None,
                ),
            ];
            if mixed {
                events.push(chunk(json!({"tool_calls":[{"index":1,"id":"actual","type":"function","function":{"name":"lookup","arguments":"{}"}}]}),None));
            }
            events.push(chunk(json!({}), Some("tool_calls")));
            let input = encoded(&events, true);
            let result = run(&settings, Value::Null, Some(&input), stream).await;
            assert!(result.error.is_none(), "{:?}", result.error);
            assert_eq!(result.body["choices"][0]["message"]["content"], "hello 😀");
            assert_eq!(
                result.body["choices"][0]["finish_reason"],
                if mixed { "tool_calls" } else { "stop" }
            );
            if mixed {
                assert_eq!(
                    result.body["choices"][0]["message"]["tool_calls"],
                    json!([actual.clone()])
                );
            }
        }
    }
}
