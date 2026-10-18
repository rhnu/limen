use serde_json::json;

use super::{GenerationParameters, project_generation_parameters, validate_parameters};

#[test]
fn projection_retains_controls_and_presence_but_excludes_content() {
    let projected = project_generation_parameters(
        &json!({"messages":[{"content":"private content"}],"temperature":null,"stream":false,"n":2,"top_p":0.8,"seed":1,"response_format":{"type":"json_schema","json_schema":{"name":"private","schema":{"private":"private"},"strict":true}},"stop":["private","secret"],"tool_choice":{"type":"function","function":{"name":"private"}},"headers":{"authorization":"private"},"user":"private"}),
    );
    assert_eq!(projected.0["temperature"], json!(null));
    assert_eq!(projected.0["stream"], false);
    assert!(!projected.0.contains_key("max_tokens"));
    assert_eq!(projected.0["stop"], json!({"count":2,"bytes":[7,6]}));
    let encoded = serde_json::to_string(&projected).unwrap();
    assert!(!encoded.contains("private"));
    assert!(!encoded.contains("secret"));
    assert!(validate_parameters(&projected));
}

#[test]
fn malformed_control_values_do_not_smuggle_strings_or_nested_content() {
    let projected = project_generation_parameters(
        &json!({"temperature":{"content":"private"},"seed":"private","service_tier":"private","stream_options":{"include_usage":{"secret":"private"}},"logit_bias":{"private":1}}),
    );
    assert!(
        !serde_json::to_string(&projected)
            .unwrap()
            .contains("private")
    );
    assert!(validate_parameters(&projected));
    assert!(!validate_parameters(&GenerationParameters(
        std::collections::BTreeMap::from([("messages".into(), json!([]))])
    )));
}

#[test]
fn large_bias_maps_keep_a_bounded_explicit_summary() {
    let bias = serde_json::Map::from_iter((0..1000).map(|index| (index.to_string(), json!(1))));
    let projected = project_generation_parameters(&json!({"logit_bias":bias}));
    assert_eq!(
        projected.0["logit_bias"],
        json!({"entries":1000,"valuesRetained":false})
    );
    assert!(validate_parameters(&projected));
}

#[test]
fn escaping_cannot_expand_a_metadata_projection_past_its_capacity() {
    let projected = project_generation_parameters(
        &json!({"model":"\u{0}".repeat(1024),"temperature":0.8,"stop":vec!["s";64]}),
    );
    assert!(validate_parameters(&projected));
    assert!(serde_json::to_vec(&projected).unwrap().len() <= 4096);
    assert_eq!(projected.0["temperature"], 0.8);
}
