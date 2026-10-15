//! Authentication, one-time routing, draft commits, and upstream failure contracts.

use anyhow::{Context, Result, ensure};
use http::Method;
use serde_json::{Value, json};

use super::fixture::{Laboratory, Options, request};

/// Check both protected endpoints, explicit models, header isolation, and single sends.
pub async fn auth_routing(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(Options::default(), async |item| {
            for (method, path) in [
                (Method::GET, "/v1/models"),
                (Method::POST, "/v1/chat/completions"),
            ] {
                item.send(request(&[]), method, path, &[("Authorization", None)])
                    .await?
                    .error(401, "unauthorized", "both endpoints must authenticate")?;
            }
            let reply = item
                .send(request(&[]), Method::GET, "/v1/models", &[])
                .await?;
            let models = reply.body["data"]
                .as_array()
                .context("model list data is not an array")?;
            ensure!(
                reply.status == 200
                    && models.iter().map(|model| &model["id"]).collect::<Vec<_>>()
                        == vec![&json!("chat")],
                "explicit model list"
            );
            item.request(request(&[("model", json!("unknown"))]))
                .await?
                .error(400, "unknown_model", "unknown route rejected")?;
            ensure!(
                item.upstream.controls.lock().await.records.is_empty(),
                "rejections must not send upstream"
            );
            let reply = item
                .send(
                    request(&[
                        ("temperature", Value::Null),
                        ("unknownExtension", json!({"keep":true})),
                    ]),
                    Method::POST,
                    "/v1/chat/completions",
                    &[
                        ("Cookie", Some("synthetic-cookie-secret=1")),
                        ("X-Unrelated", Some("hidden")),
                    ],
                )
                .await?;
            ensure!(
                reply.status == 200 && reply.body["model"] == "chat",
                "response alias"
            );
            {
                let upstream = item.upstream.controls.lock().await;
                let recorded = upstream
                    .records
                    .first()
                    .context("generation request not recorded")?;
                ensure!(
                    recorded
                        .path
                        .ends_with("/custom/completion?key=synthetic-query-secret"),
                    "complete configured URL"
                );
                ensure!(
                    recorded
                        .headers
                        .get("authorization")
                        .is_some_and(|value| value == "Bearer synthetic-upstream-secret"),
                    "independent upstream auth"
                );
                ensure!(
                    !recorded.headers.contains_key("cookie")
                        && !recorded.headers.contains_key("x-unrelated"),
                    "client headers isolated"
                );
                ensure!(
                    recorded.body["model"] == "real-model"
                        && recorded.body.get("temperature") == Some(&Value::Null)
                        && recorded.body["unknownExtension"]["keep"] == true,
                    "merge null and unknown fields"
                );
            }
            ensure!(
                !reply.headers.contains_key("set-cookie"),
                "upstream cookies excluded"
            );
            item.upstream.controls.lock().await.status = 307;
            item.request(request(&[])).await?.error(
                502,
                "upstream_failed",
                "redirect is an error",
            )?;
            ensure!(
                item.upstream.controls.lock().await.records.len() == 2,
                "one upstream send per request"
            );
            Ok(())
        })
        .await
}
