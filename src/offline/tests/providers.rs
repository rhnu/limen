use std::{fs, path::Path};

use super::{fixtures, group};
use crate::bootstrap::Loaded;

#[tokio::test(flavor = "current_thread")]
async fn builtin_provider_examples_preserve_defaults_and_all_four_transports() {
    group(&Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/providers")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn builtin_provider_scopes_share_ordered_pipeline_and_host_validation() {
    group(&fixtures("providers")).await;
}

#[tokio::test(flavor = "current_thread")]
async fn builtin_presets_cannot_bypass_host_configuration_validation() {
    let directory = tempfile::tempdir().expect("temporary provider configuration");
    let server = directory.path().join("server.toml");
    fs::write(
        &server,
        "[auth]\napi_key = 'example-limen-api-key-change-this-32-bytes'\n",
    )
    .expect("write server configuration");
    fs::copy(
        fixtures("providers").join("invalid-host.mjs"),
        directory.path().join("invalid-host.mjs"),
    )
    .expect("write invalid provider inputs");
    for factory in 0..3 {
        for case in 0..9 {
            fs::write(
                directory.path().join("gateway.js"),
                format!(
                    "import {{invalidGateway}} from './invalid-host.mjs'; \
                     export default invalidGateway({factory}, {case});"
                ),
            )
            .expect("write provider configuration");
            let error = match Loaded::load(Some(&server)).await {
                Err(error) => error,
                Ok(loaded) => {
                    loaded
                        .shutdown()
                        .await
                        .expect("stop unexpected valid configuration");
                    panic!("invalid provider configuration accepted: {factory}/{case}");
                },
            };
            assert!(matches!(
                error.phase(),
                "business_config" | "script_startup"
            ));
            let diagnostic = error.diagnostic().to_string();
            assert!(!diagnostic.contains("secret-token"));
            assert!(!diagnostic.contains("secret-getter"));
        }
    }
}
