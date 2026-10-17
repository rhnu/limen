use serde_json::json;

use super::*;

#[test]
fn each_policy_declares_its_supported_stages() {
    for (descriptor, before_send, content) in [
        (json!({"kind":"capCompletionTokens","max":10}), true, false),
        (
            json!({"kind":"dropRequestFields","fields":["x"]}),
            true,
            false,
        ),
        (
            json!({"kind":"forwardClientHeader","from":"x-a","to":"x-b"}),
            true,
            false,
        ),
        (
            json!({"kind":"replaceText","search":"a","replacement":"b","maxBufferedChars":1}),
            false,
            true,
        ),
        (
            json!({"kind":"syntheticTextTool","maxBufferedChars":128}),
            true,
            true,
        ),
    ] {
        let policy: BuiltinPolicy = serde_json::from_value(descriptor).unwrap();
        let stages = policy.stages();
        assert_eq!(stages.before_send.is_some(), before_send);
        assert_eq!(stages.content.is_some(), content);
    }
}
