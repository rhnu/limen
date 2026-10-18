use serde_json::json;

use super::{Fixture, TestResult, call, context::draft, ensure, hook, stop};
use crate::{RuntimeOptions, ScriptErrorKind, ScriptRuntime};

#[tokio::test]
async fn provider_presets_lower_credentials_headers_and_typed_options_to_plain_config() -> TestResult
{
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/providers/configuration.js"))
        .await?;
    ensure(
        config
            == json!({
                "providers": {
                    "openai": {
                        "endpoint":"https://api.openai.com/v1/chat/completions",
                        "auth":{"bearer":"synthetic-openai"},
                        "headers":{
                            "OpenAI-Organization":"synthetic-org",
                            "OpenAI-Project":"synthetic-project",
                            "X-Synthetic":"openai"
                        },
                        "defaults":{
                            "temperature":0.5,"reasoning_effort":"xhigh","verbosity":"high",
                            "service_tier":"priority","store":false,"parallel_tool_calls":true
                        },
                        "pipeline":[]
                    },
                    "deepseek": {
                        "endpoint":"https://api.deepseek.com/chat/completions",
                        "auth":{"bearer":"synthetic-deepseek"},
                        "defaults":{"thinking":{"type":"enabled"},"reasoning_effort":"max"},
                        "pipeline":[]
                    },
                    "openrouter": {
                        "endpoint":"https://openrouter.ai/api/v1/chat/completions",
                        "auth":{"bearer":"synthetic-openrouter"},
                        "headers":{
                            "HTTP-Referer":"https://synthetic.invalid/app",
                            "X-OpenRouter-Title":"Synthetic application",
                            "X-Synthetic":"openrouter"
                        },
                        "defaults":{
                            "provider":{
                                "order":["synthetic-first","synthetic-second"],
                                "only":["synthetic-only"],"ignore":["synthetic-ignore"],
                                "allow_fallbacks":false,"require_parameters":true,
                                "data_collection":"deny","sort":"latency","zdr":true
                            },
                            "reasoning":{"max_tokens":32,"exclude":false,"enabled":true}
                        },
                        "pipeline":[]
                    },
                    "custom": {
                        "endpoint":"https://synthetic.invalid/full/chat/completions?version=synthetic",
                        "auth":{"bearer":"synthetic-custom"},
                        "pipeline":[]
                    }
                },
                "models": {
                    "openai-model":{
                        "provider":"openai","upstream":"synthetic-chat",
                        "defaults":{
                            "max_completion_tokens":128,"reasoning_effort":"minimal","verbosity":"low",
                            "service_tier":"fast","store":true,"parallel_tool_calls":false
                        },
                        "pipeline":[]
                    },
                    "deepseek-model":{
                        "provider":"deepseek","upstream":"synthetic-reasoner",
                        "defaults":{"thinking":{"type":"disabled"},"reasoning_effort":"none"},
                        "pipeline":[]
                    },
                    "openrouter-model":{
                        "provider":"openrouter","upstream":"synthetic/vendor-model",
                        "defaults":{
                            "temperature":0.25,
                            "provider":{
                                "order":["synthetic-model"],"data_collection":"allow","sort":"throughput"
                            },
                            "reasoning":{"effort":"high","exclude":true,"enabled":false}
                        },
                        "pipeline":[]
                    },
                    "custom-model":{
                        "provider":"custom","upstream":"synthetic-custom","pipeline":[]
                    }
                }
            }),
        "provider preset, option lowering, or typed option snapshots changed",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn provider_scopes_compose_native_and_js_steps_and_seal_with_the_gateway() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/providers/composition.js"))
        .await?;
    let output = runtime
        .invoke_fixture(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output == json!({"frozen":true,"closedEarly":3,"sealed":15}),
        "provider helper leaked an open or mutable registration scope",
    )?;
    for index in 0..3 {
        let provider = &config["providers"][format!("provider-{index}")];
        let model = &config["models"][format!("model-{index}")];
        ensure(
            provider["pipeline"][0]
                == json!({"name":"cap","builtin":{"kind":"capCompletionTokens","max":16}})
                && model["pipeline"][0] == provider["pipeline"][0]
                && provider["pipeline"]
                    .as_array()
                    .is_some_and(|steps| steps.len() == 2)
                && model["pipeline"]
                    .as_array()
                    .is_some_and(|steps| steps.len() == 2),
            "provider helper changed explicit pipeline declarations",
        )?;
        let mut input = draft("beforeSend");
        input["state"] = json!({"trace":[]});
        for step in [&provider["pipeline"][1], &model["pipeline"][1]] {
            let output = runtime
                .invoke_fixture(call(hook(step, "beforeSend")?, 2, input.clone()))
                .await
                .into_context()?;
            input["state"] = output["state"].clone();
            input["body"] = output["body"].clone();
        }
        ensure(
            input["state"]["trace"] == json!(["provider", "model"])
                && input["body"]["provider"] == true
                && input["body"]["model"] == "synthetic-overridden",
            "provider helper changed normal sync or async hook behavior",
        )?;
    }
    stop(&runtime).await
}

#[tokio::test]
async fn provider_helpers_reject_malformed_options_and_conflicts_without_running_getters()
-> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/providers/invalid-options.js"))
        .await?;
    let output = runtime
        .invoke_fixture(call(hook(&config, "onRequest")?, 1, json!({})))
        .await
        .into_context()?;
    ensure(
        output["failures"] == json!([]) && output["touched"] == 0,
        &format!("provider validation accepted an invalid option or executed a getter: {output}"),
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn provider_helpers_accept_every_supported_enum_and_preserve_raw_extensions() -> TestResult {
    let fixture = Fixture::new()?;
    let (runtime, config) = fixture
        .start(include_str!("../../fixtures/providers/accepted-options.js"))
        .await?;
    for effort in ["none", "minimal", "low", "medium", "high", "xhigh", "max"] {
        for provider in ["openai", "deepseek", "openrouter"] {
            let id = format!("{provider}-{effort}");
            let expected = if provider == "openrouter" {
                json!({"reasoning":{"effort":effort}})
            } else {
                json!({"reasoning_effort":effort})
            };
            ensure(
                config["providers"][&id]["defaults"] == expected
                    && config["models"][&id]["defaults"] == expected,
                "supported reasoning effort did not lower identically at provider and model layers",
            )?;
        }
    }
    for tier in ["auto", "default", "flex", "scale", "priority", "fast"] {
        ensure(
            config["providers"][format!("tier-{tier}")]["defaults"] == json!({"service_tier":tier}),
            "supported OpenAI service tier was changed",
        )?;
    }
    for verbosity in ["low", "medium", "high"] {
        ensure(
            config["providers"][format!("verbosity-{verbosity}")]["defaults"]
                == json!({"verbosity":verbosity}),
            "supported OpenAI verbosity was changed",
        )?;
    }
    for sort in ["price", "throughput", "latency"] {
        ensure(
            config["providers"][format!("sort-{sort}")]["defaults"]
                == json!({"provider":{"sort":sort}}),
            "supported OpenRouter provider sort was changed",
        )?;
    }
    ensure(
        config["providers"]["maximum"]["defaults"]["reasoning"]["max_tokens"]
            == 9_007_199_254_740_991_u64
            && config["providers"]["raw-headers"]["headers"]
                == json!({"OpenAI-Organization":"synthetic-org","OpenAI-Project":"synthetic-project"})
            && config["providers"]["raw-headers"]["defaults"]
                == json!({"vendor_extension":{"enabled":true}})
            && config["providers"]["raw-attribution"]["headers"]
                == json!({"HTTP-Referer":"https://synthetic.invalid","X-OpenRouter-Title":"Synthetic"})
            && config["providers"]["raw-attribution"]["endpoint"]
                == "https://synthetic.invalid/router/chat"
            && config["providers"]["custom-deepseek"]["endpoint"]
                == "https://synthetic.invalid/deepseek/chat"
            && config["providers"]["duplicate-raw-headers"]["headers"]
                == json!({"X-Synthetic":"first","x-synthetic":"second"}),
        "safe integer boundary, raw extension, optional attribution header, or full endpoint changed",
    )?;
    stop(&runtime).await
}

#[tokio::test]
async fn provider_module_reuses_esm_identity_and_exposes_only_public_factories() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write("providers.js", "export const createOpenAI = 'local';")?;
    fixture.write(
        "reexport.mjs",
        "export {createOpenAI} from 'limen:providers';",
    )?;
    let path = fixture.write(
        "gateway.js",
        include_str!("../../fixtures/providers/module-cache.js"),
    )?;
    let (runtime, config) = ScriptRuntime::start(
        path,
        RuntimeOptions {
            max_modules: 4,
            ..RuntimeOptions::default()
        },
    )
    .await?;
    ensure(
        config
            == json!({
                "same":true,"local":"local",
                "exports":["createDeepSeek","createOpenAI","createOpenRouter"]
            }),
        "provider module identity, private exports, or reserved module resolution changed",
    )?;
    stop(&runtime).await?;
    for source in [
        "import {missing} from 'limen:providers'; export default () => ({});",
        "import 'limen:providers/extra'; export default () => ({});",
        "import 'limen:providers?query'; export default () => ({});",
        "import 'limen:providers#fragment'; export default () => ({});",
        "import 'limen:providers/internal/options'; export default () => ({});",
    ] {
        match fixture.start(source).await {
            Err(error) => ensure(
                error
                    .downcast_ref::<crate::ScriptError>()
                    .is_some_and(|error| error.kind() == ScriptErrorKind::Module),
                "invalid provider module import changed its controlled failure",
            )?,
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err("invalid provider module import was accepted".into());
            },
        }
    }
    Ok(())
}

#[tokio::test]
async fn provider_startup_failures_do_not_expose_credentials_or_option_values() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write(
        "cases.mjs",
        include_str!("../../fixtures/providers/startup-cases.mjs"),
    )?;
    for index in 0..4 {
        let source = format!("import {{cases}} from './cases.mjs'; export default cases[{index}];");
        match fixture.start(&source).await {
            Err(error) => {
                let error = error
                    .downcast_ref::<crate::ScriptError>()
                    .ok_or("provider startup failure lost its script error")?;
                let diagnostic = error.diagnostic().to_string();
                ensure(
                    error.kind() == ScriptErrorKind::Json
                        && diagnostic.contains("exception=TypeError")
                        && !diagnostic.contains("synthetic-secret")
                        && !diagnostic.contains("apiKey")
                        && !diagnostic.contains("reasoningEffort"),
                    "provider startup failure lost its controlled class or exposed script data",
                )?;
            },
            Ok((runtime, _)) => {
                stop(&runtime).await?;
                return Err(format!("invalid provider startup case {index} was accepted").into());
            },
        }
    }
    Ok(())
}
