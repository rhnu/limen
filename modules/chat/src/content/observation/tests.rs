use serde_json::json;

use super::*;

#[test]
fn metadata_and_empty_fragments_do_not_mark_generated_content() {
    for delta in [
        json!({}),
        json!({"role":"assistant"}),
        json!({"content":""}),
        json!({"content":null}),
        json!({"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"","arguments":""}}]}),
        json!({"audio":{"id":"audio-1","expires_at":10}}),
    ] {
        let facts = observe_generated_content(
            &json!({"choices":[{"index":0,"delta":delta,"finish_reason":"stop"}]}),
        );
        assert!(!facts.generation);
        assert!(!facts.text);
    }
    assert!(
        !observe_generated_content(&json!({"usage":{"completion_tokens":2},"choices":[]}))
            .generation
    );
}

#[test]
fn recognized_content_matches_text_reasoning_refusal_tools_and_audio_semantics() {
    for (delta, text, reasoning, audio) in [
        (json!({"content":"answer"}), true, false, false),
        (
            json!({"content":[{"type":"text","text":"answer"}]}),
            true,
            false,
            false,
        ),
        (json!({"reasoning_content":"thinking"}), false, true, false),
        (
            json!({"reasoning_details":[{"type":"reasoning.text","text":"thinking"}]}),
            false,
            true,
            false,
        ),
        (
            json!({"content":[{"type":"thinking","thinking":[{"type":"text","text":"thinking"}]}]}),
            false,
            true,
            false,
        ),
        (json!({"refusal":"no"}), false, false, false),
        (
            json!({"tool_calls":[{"index":0,"function":{"name":"search"}}]}),
            false,
            false,
            false,
        ),
        (
            json!({"function_call":{"arguments":"{}"}}),
            false,
            false,
            false,
        ),
        (
            json!({"tool_calls":[{"index":0,"custom":{"input":"query"}}]}),
            false,
            false,
            false,
        ),
        (json!({"audio":{"data":"AQ=="}}), false, false, true),
    ] {
        let facts = observe_generated_content(&json!({"choices":[{"index":0,"delta":delta}]}));
        assert_eq!(
            facts,
            GeneratedContent {
                generation: true,
                text,
                reasoning,
                audio,
                qualified: true
            }
        );
    }
}

#[test]
fn opaque_fields_only_disqualify_metrics_and_preserve_understood_content() {
    let facts = observe_generated_content(
        &json!({"choices":[{"index":0,"delta":{"content":"answer","vendor_generated":{"opaque":true}}}]}),
    );
    assert!(facts.generation);
    assert!(facts.text);
    assert!(!facts.qualified);
    let facts = observe_generated_content(
        &json!({"choices":[{"index":0,"delta":{"reasoning_details":[{"type":"reasoning.encrypted","data":"opaque"}]}}]}),
    );
    assert!(!facts.generation);
    assert!(!facts.qualified);
}
