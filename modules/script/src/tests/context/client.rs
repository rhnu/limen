use serde_json::json;

use super::draft;
use crate::{
    ScriptErrorKind,
    tests::{Fixture, TestResult, call, ensure, hook, stop},
};

#[tokio::test]
async fn readonly_client_headers_preserve_values_and_owned_snapshot() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../../fixtures/context/client.js"))
        .await?;
    let id = hook(&config, "onRequest")?;
    let client = json!({"headers": [
        {"name": "X-Example", "value": "first"},
        {"name": "x-example", "value": "second,third"},
        {"name": "X-EXAMPLE", "value": ""}
    ]});
    for phase in [
        "onRequest",
        "beforeSend",
        "afterReceive",
        "onResponse",
        "onUpstreamEvent",
        "onStreamEvent",
        "onError",
        "onFinish",
    ] {
        let mut context = draft(phase);
        context["client"] = client.clone();
        context
            .as_object_mut()
            .ok_or("context was not an object")?
            .remove("headers");
        let output = runtime.invoke(call(id, 1, context.clone())).await?;
        ensure(
            output["client"] == client,
            "client header lookup or copied values changed the input snapshot",
        )?;
        if matches!(phase, "onError" | "onFinish") {
            ensure(
                output == context,
                "observer client view changed the final context",
            )?;
        } else {
            ensure(
                output["state"]["read"] == true,
                "client view was not readable during the hook",
            )?;
        }
        for operation in [
            "client-ref",
            "headers-ref",
            "method-ref",
            "method-object",
            "new-field",
            "append",
            "set",
            "delete",
        ] {
            let mut context = context.clone();
            context["meta"]["operation"] = json!(operation);
            ensure(
                runtime
                    .invoke(call(id, 2, context))
                    .await
                    .is_err_and(|error| error.kind() == ScriptErrorKind::JavaScript),
                "readonly client header view allowed mutation",
            )?;
        }
    }
    let mut context = draft("beforeSend");
    context["client"] = client.clone();
    let output = runtime.invoke(call(id, 3, context)).await?;
    ensure(
        output["client"] == client
            && output["headers"][2] == json!({"name": "x-upstream", "value": "independent"}),
        "client view interfered with the mutable upstream header draft",
    )?;
    stop(&runtime).await
}
