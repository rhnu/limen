use super::executable;

#[test]
fn selects_application_artifact_without_assuming_target_directory() {
    let messages = r#"{"reason":"compiler-artifact","target":{"name":"dependency","kind":["bin"]},"executable":"wrong"}
{"reason":"compiler-artifact","target":{"name":"limen","kind":["bin"]},"profile":{"test":true},"executable":"test-harness"}
{"reason":"compiler-artifact","target":{"name":"limen","kind":["bin"]},"profile":{"test":false},"executable":"custom output/debug/limen.exe"}"#;
    assert_eq!(
        executable(messages, "limen").expect("application artifact"),
        std::path::PathBuf::from("custom output/debug/limen.exe")
    );
}

#[test]
fn rejects_missing_and_malformed_artifacts() {
    assert!(executable(r#"{"reason":"build-finished","success":true}"#, "limen").is_err());
    let error = executable("invalid-json", "limen").expect_err("malformed message");
    assert!(error.downcast_ref::<serde_json::Error>().is_some());
}
