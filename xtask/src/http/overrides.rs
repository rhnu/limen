//! Request policy is checked at the upstream HTTP boundary, using the real gateway binary.

use anyhow::{Context, Result, ensure};
use serde_json::json;

use super::fixture::{Laboratory, Options, request};

const CONFIG: &str = r"export default {
 onRequest(ctx) {
  if (ctx.body.model === 'incoming') ctx.body.model = 'chat';
  if (ctx.body.enforce) ctx.body.top_p = 0.75;
 },
 providers: {
  primary: {endpoint: ENDPOINT, auth: {bearer: 'synthetic-upstream-secret'},
   headers: {'X-Policy': 'static', 'X-Remove': 'static'},
   defaults: {temperature: 0.2, top_p: 0.9, max_completion_tokens: 128,
    reasoning: {effort: 'low', summary: 'auto'}, stop: ['provider', 'shared']},
   pipeline: [
    {name: 'compatibility', beforeSend(ctx) {
     delete ctx.body.store;
     ctx.headers.delete('x-remove');
     if (ctx.body.enforce) {
      ctx.body.temperature = 0.6;
      ctx.body.max_completion_tokens += 128;
      ctx.body.model = 'decoy';
      ctx.headers.set('x-policy', 'provider');
     }
    }},
    {name: 'provider-policy', async beforeSend(ctx) {
     await Promise.resolve();
     if (ctx.body.enforce) {
      ctx.body.temperature = 0.5;
      ctx.headers.set('X-Policy', 'provider-final');
     }
    }}
   ]},
  decoy: {endpoint: ENDPOINT, headers: {'X-Wrong-Route': 'yes'}}
 },
 models: {
  chat: {provider: 'primary', upstream: 'real-model',
   defaults: {temperature: 0.4, max_completion_tokens: 256,
    reasoning: {effort: 'medium'}, stop: ['model']},
   pipeline: [
    {name: 'model-policy', async beforeSend(ctx) {
     await Promise.resolve();
     if (ctx.body.enforce) {
      ctx.body.temperature = 0.3;
      ctx.body.max_completion_tokens *= 2;
      ctx.headers.set('X-Policy', 'model');
      ctx.headers.delete('AUTHORIZATION');
      ctx.url = ctx.url.replace('/custom/completion', '/model/completion');
     }
    }},
    {name: 'final-policy', beforeSend(ctx) {
     if (ctx.body.enforce) {
      ctx.body.temperature = 0.1;
      ctx.headers.set('x-policy', 'model-final');
      ctx.headers.append('X-Labels', 'one');
      ctx.headers.append('x-labels', 'two');
     }
    }}
   ]},
  decoy: {provider: 'decoy', upstream: 'wrong-model'}
 }
};";

/// Cover defaults, shallow replacement, explicit nulls and enforced policy after client input.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(CONFIG),
                ..Options::default()
            },
            async |item| {
                let scenarios = [
                    (
                        "missing fields use model defaults before provider defaults",
                        json!({}),
                        json!({"temperature":0.4,"top_p":0.9,"max_completion_tokens":256,
                            "reasoning":{"effort":"medium"},"stop":["model"]}),
                    ),
                    (
                        "client objects and arrays replace defaults as whole values",
                        json!({"temperature":0.8,"max_completion_tokens":7,
                            "reasoning":{"summary":"detailed"},"stop":["client"]}),
                        json!({"temperature":0.8,"top_p":0.9,"max_completion_tokens":7,
                            "reasoning":{"summary":"detailed"},"stop":["client"]}),
                    ),
                    (
                        "explicit nulls do not restore defaults",
                        json!({"temperature":null,"max_completion_tokens":null,
                            "reasoning":null,"stop":null}),
                        json!({"temperature":null,"top_p":0.9,"max_completion_tokens":null,
                            "reasoning":null,"stop":null}),
                    ),
                    (
                        "request hooks override client values without rerouting",
                        json!({"enforce":true,"temperature":0.8,"max_completion_tokens":7}),
                        json!({"enforce":true,"temperature":0.1,"top_p":0.75,
                            "max_completion_tokens":270,"reasoning":{"effort":"medium"},
                            "stop":["model"]}),
                    ),
                ];
                for (index, (meaning, fields, expected)) in scenarios.into_iter().enumerate() {
                    let mut body = request(&[
                        ("model", json!("incoming")),
                        ("store", json!(true)),
                        ("messages", json!([{"role":"user","content":[
                            {"type":"text","text":"synthetic-body-secret"},
                            {"type":"image_url","image_url":{"url":"https://image.invalid/picture"}}
                        ]}])),
                        ("tools", json!([{"type":"function","function":{
                            "name":"lookup","parameters":{"type":"object","properties":{}}
                        }}])),
                        ("vendor_extension", json!({"enabled":false,"options":[1,null]})),
                    ]);
                    body.as_object_mut()
                        .context("scenario request object")?
                        .extend(fields.as_object().context("scenario fields object")?.clone());
                    let reply = item.request(body.clone()).await?;
                    ensure!(reply.status == 200 && reply.body["model"] == "chat", "{meaning}: successful response uses selected alias");
                    let upstream = item.upstream.controls.lock().await;
                    ensure!(upstream.records.len() == index + 1, "{meaning}: exactly one upstream request");
                    let recorded = upstream.records.last().context("override request missing")?;
                    let mut expected = expected;
                    for field in ["messages", "tools", "vendor_extension"] {
                        expected[field] = body[field].clone();
                    }
                    let enforced = body["enforce"] == true;
                    expected["model"] = json!(if enforced { "decoy" } else { "real-model" });
                    ensure!(recorded.body == expected, "{meaning}: final upstream body");
                    ensure!(
                        recorded.path == format!("{}?key=synthetic-query-secret", if enforced { "/model/completion" } else { "/custom/completion" }),
                        "{meaning}: complete final endpoint"
                    );
                    ensure!(
                        !recorded.headers.contains_key("x-remove")
                            && !recorded.headers.contains_key("x-wrong-route")
                            && recorded.headers.get("x-policy").is_some_and(|value| value == if enforced { "model-final" } else { "static" }),
                        "{meaning}: final header policy and original provider retained"
                    );
                    if enforced {
                        ensure!(!recorded.headers.contains_key("authorization"), "removed upstream auth stays removed");
                        let labels = recorded.headers.get_all("x-labels").iter()
                            .map(http::HeaderValue::to_str)
                            .collect::<std::result::Result<Vec<_>, _>>()
                            .context("read final multivalue header")?;
                        ensure!(labels == ["one", "two"], "appended header values reach upstream separately");
                    } else {
                        ensure!(recorded.headers.get("authorization").is_some_and(|value| value == "Bearer synthetic-upstream-secret"), "configured upstream auth reaches upstream");
                    }
                }
                Ok(())
            },
        )
        .await
}
