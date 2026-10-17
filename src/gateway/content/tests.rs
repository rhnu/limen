use super::*;

#[test]
fn retained_grammar_budget_is_shared_by_source_and_all_steps() {
    let mut pipeline = ContentPipeline {
        source: ContentProjector::new(100),
        stages: vec![ContentProjector::new(100); 2],
    };
    let frame = pipeline
        .source
        .project_chunk(json!({"choices":[{"index":0,"delta":{"content":"a"}}]}))
        .unwrap();
    for stage in &mut pipeline.stages {
        stage
            .render(frame.clone(), frame.events().to_vec())
            .unwrap();
    }
    let mut limits = ResourceLimits::default();
    pipeline.capacity(None, &limits).unwrap();
    limits.max_output_bytes = pipeline
        .source
        .retained_bytes()
        .max(pipeline.stages[0].retained_bytes());
    assert_eq!(
        pipeline.capacity(None, &limits).unwrap_err().code,
        ErrorCode::HookFailed
    );
    limits.max_output_bytes = usize::MAX;
    limits.max_json_nodes = pipeline.source.retained_parts();
    assert_eq!(
        pipeline.capacity(None, &limits).unwrap_err().code,
        ErrorCode::HookFailed
    );
}
