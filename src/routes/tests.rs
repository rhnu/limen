use super::*;

#[test]
fn namespaces_are_exact_and_unknown_members_keep_their_authentication_class() {
    for path in ["/v1", "/v1/", "/v1/unknown", "/v1/models/"] {
        assert_eq!(classify(path), Route::Inference(InferenceRoute::Unknown));
    }
    for path in [
        "/admin/v1",
        "/admin/v1/",
        "/admin/v1/requests/",
        "/admin/v1/requests/a/b",
    ] {
        assert_eq!(classify(path), Route::Management(AdminRoute::Unknown));
    }
    for path in [
        "/",
        "/v10/models",
        "/admin/v10/status",
        "/assets/app.js",
        "/usage",
    ] {
        assert_eq!(classify(path), Route::Public);
    }
    assert_eq!(
        classify("/v1/chat/completions"),
        Route::Inference(InferenceRoute::Chat)
    );
    assert_eq!(
        classify("/v1/models"),
        Route::Inference(InferenceRoute::Models)
    );
    assert_eq!(
        classify("/admin/v1/requests/request-1"),
        Route::Management(AdminRoute::Request("request-1".into()))
    );
    assert_eq!(
        classify("/admin/v1/logs/files"),
        Route::Management(AdminRoute::LogFiles)
    );
}
