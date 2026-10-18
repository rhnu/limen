import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";
import { capCompletionTokens } from "limen:policies";

/** @typedef {{count?: number}} State */
/** Preserve both generic dimensions when composing reusable model registrations.
 * @template {import("../limen").JsonObject} State
 * @template Options
 * @param {import("limen:gateway").ProviderScope<State, Options>} provider
 * @param {import("limen:gateway").ModelOptions<Options>} options
 * @returns {import("limen:gateway").ModelScope<State>}
 */
function register(provider, options) {
  return provider.model("helper", options);
}

/** @type {import("limen:gateway").GatewaySetup<State>} */
const setup = (gateway) => {
  const openai = createOpenAI(gateway, "openai", {
    apiKey: "example",
    organization: "organization",
    project: "project",
    headers: { "X-Example": "value" },
    defaults: { temperature: 0.5, extension: { key: true } },
    options: {
      reasoningEffort: "high",
      verbosity: "low",
      serviceTier: "priority",
      store: false,
      parallelToolCalls: true,
    },
  });
  /** @type {import("limen:gateway").ProviderScope<State, import("limen:providers").OpenAIOptions>} */
  const inferredOpenAI = openai;
  inferredOpenAI.model("openai-model", {
    upstream: "model",
    defaults: { temperature: 0.25 },
    options: { reasoningEffort: "max", store: true },
  });
  register(openai, { upstream: "model", options: { verbosity: "high" } });
  // @ts-expect-error Generic registration must retain the provider's options type.
  register(openai, { upstream: "model", options: { thinking: "enabled" } });
  /** @type {import("limen:gateway").ModelOptions<import("limen:providers").OpenAIOptions>} */
  const annotatedOptions = { upstream: "model", options: { serviceTier: "fast" } };
  openai.model("annotated", annotatedOptions);
  openai.use(capCompletionTokens({ name: "budget", max: 64 }));
  openai.step("state", (step) => {
    step.beforeSend((ctx) => {
      ctx.state.count = 1;
      // @ts-expect-error Factories preserve the gateway's state type.
      ctx.state.count = "wrong";
    });
  });
  openai.model("state-model", { upstream: "model" }).step("state", (step) => {
    step.beforeSend((ctx) => {
      ctx.state.count = 1;
      // @ts-expect-error Model hooks preserve the gateway's state type.
      ctx.state.count = false;
    });
  });

  const deepseek = createDeepSeek(gateway, "deepseek", {
    apiKey: "example",
    endpoint: "https://fictional.invalid/complete-chat-path",
    options: { thinking: "enabled", reasoningEffort: "xhigh" },
  });
  /** @type {import("limen:gateway").ProviderScope<State, import("limen:providers").DeepSeekOptions>} */
  const inferredDeepSeek = deepseek;
  inferredDeepSeek.model("deepseek-model", {
    upstream: "model",
    options: { thinking: "disabled", reasoningEffort: "none" },
  });
  // @ts-expect-error Generic registration cannot widen DeepSeek options to another provider.
  register(deepseek, { upstream: "model", options: { verbosity: "low" } });
  deepseek.step("state", (step) =>
    step.beforeSend((ctx) => {
      ctx.state.count = 1;
      // @ts-expect-error DeepSeek retains State.
      ctx.state.count = "wrong";
    }),
  );

  const openrouter = createOpenRouter(gateway, "openrouter", {
    apiKey: "example",
    appUrl: "https://fictional.invalid/",
    appTitle: "Example",
    options: {
      provider: {
        order: ["provider"],
        only: ["provider"],
        ignore: ["other"],
        allowFallbacks: false,
        requireParameters: true,
        zdr: true,
        dataCollection: "deny",
        sort: "latency",
      },
      reasoning: { effort: "minimal", exclude: true, enabled: true },
    },
  });
  /** @type {import("limen:gateway").ProviderScope<State, import("limen:providers").OpenRouterOptions>} */
  const inferredOpenRouter = openrouter;
  inferredOpenRouter.model("router-model", {
    upstream: "model",
    options: { reasoning: { maxTokens: 1024, enabled: true }, provider: { sort: "price" } },
  });
  openrouter.step("state", (step) =>
    step.beforeSend((ctx) => {
      ctx.state.count = 1;
      // @ts-expect-error OpenRouter retains State.
      ctx.state.count = "wrong";
    }),
  );

  // @ts-expect-error Factories require a non-optional apiKey.
  createOpenAI(gateway, "bad", {});
  // @ts-expect-error Unknown factory configuration keys are rejected.
  createDeepSeek(gateway, "bad", { apiKey: "example", retry: 2 });
  // @ts-expect-error Dedicated OpenAI headers are vendor-specific.
  createDeepSeek(gateway, "bad", { apiKey: "example", organization: "organization" });
  // @ts-expect-error Credentials do not accept explicit undefined.
  createOpenAI(gateway, "bad", { apiKey: undefined });
  // @ts-expect-error Optional factory fields must be omitted rather than undefined.
  createOpenAI(gateway, "bad", { apiKey: "example", endpoint: undefined });
  // @ts-expect-error Options must be omitted rather than undefined.
  createOpenAI(gateway, "bad", { apiKey: "example", options: undefined });
  // @ts-expect-error OpenAI accepts only its listed effort values.
  createOpenAI(gateway, "bad", { apiKey: "example", options: { reasoningEffort: "unlimited" } });
  // @ts-expect-error Wire spelling belongs in raw defaults, not typed options.
  createOpenAI(gateway, "bad", { apiKey: "example", options: { reasoning_effort: "high" } });
  // @ts-expect-error Dedicated options are not an open JSON bag.
  openai.model("bad", { upstream: "model", options: { temperature: 0.5 } });
  // @ts-expect-error DeepSeek fields cannot be used on OpenAI models.
  openai.model("bad", { upstream: "model", options: { thinking: "enabled" } });
  // @ts-expect-error OpenAI fields cannot be used on DeepSeek models.
  deepseek.model("bad", { upstream: "model", options: { verbosity: "low" } });
  // @ts-expect-error DeepSeek thinking is a fixed enum.
  deepseek.model("bad", { upstream: "model", options: { thinking: true } });
  // @ts-expect-error Booleans do not accept strings.
  openai.model("bad", { upstream: "model", options: { store: "false" } });
  // @ts-expect-error Service tiers are a fixed enum.
  openai.model("bad", { upstream: "model", options: { serviceTier: "premium" } });
  // @ts-expect-error Typed optional values must be omitted rather than undefined.
  openai.model("bad", { upstream: "model", options: { reasoningEffort: undefined } });
  // @ts-expect-error Model options must be omitted rather than undefined.
  openai.model("bad", { upstream: "model", options: undefined });
  // @ts-expect-error Unknown OpenRouter option keys are rejected.
  openrouter.model("bad", { upstream: "model", options: { store: false } });
  // @ts-expect-error Nested routing options are closed.
  openrouter.model("bad", { upstream: "model", options: { provider: { arbitrary: true } } });
  // @ts-expect-error Routing arrays contain strings.
  openrouter.model("bad", { upstream: "model", options: { provider: { order: [1] } } });
  openrouter.model("bad", {
    upstream: "model",
    // @ts-expect-error Nested routing enum is fixed.
    options: { provider: { dataCollection: "maybe" } },
  });
  // @ts-expect-error Nested routing values must be omitted rather than undefined.
  openrouter.model("bad", { upstream: "model", options: { provider: { sort: undefined } } });
  // @ts-expect-error Reasoning object does not accept wire spelling.
  openrouter.model("bad", { upstream: "model", options: { reasoning: { max_tokens: 128 } } });
  openrouter.model("bad", {
    upstream: "model",
    // @ts-expect-error Effort and token budget are mutually exclusive.
    options: { reasoning: { effort: "low", maxTokens: 128 } },
  });
  // @ts-expect-error Nested reasoning values must be omitted rather than undefined.
  openrouter.model("bad", { upstream: "model", options: { reasoning: { maxTokens: undefined } } });
  // @ts-expect-error Raw provider registration has no vendor options.
  gateway.provider("raw", { endpoint: "https://fictional.invalid/", options: { store: true } });
  const raw = gateway.provider("raw-models", { endpoint: "https://fictional.invalid/" });
  // @ts-expect-error Raw provider model registration has no vendor options.
  raw.model("bad", { upstream: "model", options: { store: true } });
  // @ts-expect-error The default generic also forbids explicitly undefined options.
  raw.model("bad", { upstream: "model", options: undefined });
  /** @type {import("limen:gateway").ProviderScope<State>} */
  const oldGenericCaller = raw;
  /** @type {import("limen:gateway").ModelOptions} */
  const oldModelOptions = { upstream: "model", defaults: { temperature: 0.25 } };
  oldGenericCaller.model("old", oldModelOptions);
};
export const providers = defineGateway(setup);
