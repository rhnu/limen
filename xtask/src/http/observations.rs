//! Terminal observers cannot replace outcomes, recurse into onError, or wait indefinitely.

use std::time::Duration;

use anyhow::{Result, ensure};
use serde_json::json;
use tokio::time::Instant;

use super::fixture::{Laboratory, Options, request};

const CONFIG: &str = r"let errors=0,finishes=0;
export default {
 onRequest(ctx){ctx.state.mode=ctx.body.mode||'normal';},
 onError(ctx){errors++;if(ctx.state.mode==='error-wait')return new Promise(()=>{});throw new Error('synthetic-body-secret');},
 onFinish(ctx){finishes++;if(ctx.state.mode==='finish-wait')return new Promise(()=>{});if(ctx.state.mode==='bad')throw new Error('synthetic-body-secret');},
 providers:{p:{endpoint:ENDPOINT,pipeline:[{name:'probe',
  beforeSend(ctx){if(ctx.state.mode==='bad'||ctx.state.mode==='error-wait')throw new Error('synthetic-body-secret');},
  onResponse(ctx){ctx.body.counts=[errors,finishes];}
 }]}},models:{chat:{provider:'p',upstream:'real-model'}}};";

/// Correlate observer failures and cleanup exhaustion with the original request IDs.
pub async fn run(laboratory: &mut Laboratory<'_>) -> Result<()> {
    laboratory
        .with_fixture(
            Options {
                config: Some(CONFIG),
                ..Options::default()
            },
            async |item| {
                let reply = item.request(request(&[("mode", json!("bad"))])).await?;
                reply.error(
                    500,
                    "hook_failed",
                    "failed observers preserve original failure",
                )?;
                let request_id = &reply.body["error"]["request_id"];
                item.wait_event(|event| {
                    event["request_id"] == *request_id
                        && event["phase"] == "onFinish"
                        && event["error_kind"] == "JavaScript"
                })
                .await?;
                let reply = item.request(request(&[])).await?;
                ensure!(
                    reply.status == 200 && reply.body["counts"] == json!([1, 1]),
                    "failed observers run once without recursive onError"
                );

                let started = Instant::now();
                let reply = item
                    .request(request(&[("mode", json!("error-wait"))]))
                    .await?;
                reply.error(
                    500,
                    "hook_failed",
                    "pending error observer preserves failure",
                )?;
                ensure!(
                    started.elapsed() < Duration::from_millis(800),
                    "error observer cleanup deadline is finite"
                );
                let request_id = &reply.body["error"]["request_id"];
                item.wait_event(|event| {
                    event["request_id"] == *request_id
                        && event["phase"] == "onFinish"
                        && event["reason"] == "cleanup_budget"
                })
                .await?;
                let reply = item.request(request(&[])).await?;
                ensure!(
                    reply.status == 200 && reply.body["counts"] == json!([2, 2]),
                    "expired onError leaves onFinish unexecuted"
                );

                let started = Instant::now();
                let reply = item
                    .request(request(&[("mode", json!("finish-wait"))]))
                    .await?;
                ensure!(
                    reply.status == 200 && started.elapsed() < Duration::from_millis(800),
                    "pending finish observer preserves bounded success"
                );
                let request_id = reply.header("x-limen-request-id")?;
                item.wait_event(|event| {
                    event["request_id"] == request_id
                        && event["phase"] == "onFinish"
                        && event["error_kind"] == "Deadline"
                })
                .await?;
                let reply = item.request(request(&[])).await?;
                ensure!(
                    reply.status == 200 && reply.body["counts"] == json!([2, 4]),
                    "finish observer timeout never invokes onError"
                );
                ensure!(
                    item.events()?
                        .iter()
                        .any(|event| event["phase"] == "onFinish"
                            && event["reason"] == "cleanup_budget"),
                    "exhausted cleanup reports skipped finish"
                );
                Ok(())
            },
        )
        .await
}
