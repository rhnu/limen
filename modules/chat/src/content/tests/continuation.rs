use super::*;

#[test]
fn converted_tool_continuation_renders_text_when_event_json_is_unchanged() {
    let mut projector = ContentProjector::new(16);
    let first = projector.project_chunk(json!({"choices":[{"index":0,"delta":{
        "tool_calls":[{"index":0,"type":"function","function":{"name":"emit_text","arguments":"{\"text\":\"A"}}]
    }}]})).unwrap();
    let events = vec![
        json!({"type":"part_start","choice":0,"part":"tool:0","kind":"text"}),
        json!({"type":"part_delta","choice":0,"part":"tool:0","text":"A"}),
    ];
    projector.render(first, &events).unwrap();
    let next = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{
            "content":"B","tool_calls":[{"index":0,"function":{"arguments":"C"}}]
        }}]}))
        .unwrap();
    let events = next.events().to_vec();
    let output = projector.render(next, &events).unwrap();
    assert_eq!(output["choices"][0]["delta"]["content"], "BC");
    assert!(output["choices"][0]["delta"].get("tool_calls").is_none());
}

#[test]
fn reindexed_tool_continuation_keeps_the_committed_output_index() {
    let mut projector = ContentProjector::new(16);
    let first = projector.project_chunk(json!({"choices":[{"index":0,"delta":{
        "tool_calls":[{"index":7,"type":"function","function":{"name":"lookup","arguments":"{"}}]
    }}]})).unwrap();
    let mut events = first.events().to_vec();
    events[0]["tool"]["index"] = json!(0);
    projector.render(first, &events).unwrap();
    let next = projector
        .project_chunk(json!({"choices":[{"index":0,"delta":{
            "tool_calls":[{"index":7,"function":{"arguments":"}"}}]
        }}]}))
        .unwrap();
    let events = next.events().to_vec();
    let output = projector.render(next, &events).unwrap();
    assert_eq!(output["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
    assert_eq!(
        output["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "}"
    );
}
