import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";

let touched = 0;
const accessor = (key) =>
  Object.defineProperty({}, key, {
    enumerable: true,
    get() {
      touched++;
      return "synthetic-secret";
    },
  });

export default defineGateway((gateway) => {
  const failures = [];
  let checked = 0;
  function reject(label, operation) {
    checked++;
    try {
      operation();
      failures.push(`${label}: accepted`);
    } catch (error) {
      if (!(error instanceof TypeError)) failures.push(`${label}: wrong exception`);
    }
  }
  let next = 0;
  for (const createProvider of [createOpenAI, createDeepSeek, createOpenRouter]) {
    function create(config) {
      return createProvider(gateway, `case-${next++}`, config);
    }
    for (const value of [
      undefined,
      null,
      [],
      1,
      "synthetic-secret",
      Object.create({ apiKey: "inherited" }),
      { apiKey: "synthetic", [Symbol()]: true },
      accessor("apiKey"),
    ]) {
      reject("config shape", () => create(value));
    }
    for (const apiKey of [undefined, null, "", 1, false]) {
      reject("api key", () => create({ apiKey }));
    }
    reject("missing api key", () => create({}));
    for (const field of ["endpoint", "headers", "defaults", "options"]) {
      reject(`${field} undefined`, () => create({ apiKey: "synthetic", [field]: undefined }));
    }
    reject("unknown config option", () => create({ apiKey: "synthetic", unknown: true }));
    for (const headers of [
      { Authorization: "synthetic-secret" },
      { aUtHoRiZaTiOn: "synthetic-secret" },
      accessor("X-Synthetic"),
    ]) {
      reject("headers", () => create({ apiKey: "synthetic", headers }));
    }
    for (const options of [
      undefined,
      null,
      [],
      1,
      { unknown: true },
      { [Symbol()]: true },
      Object.create({ unknown: true }),
    ]) {
      reject("options shape", () => create({ apiKey: "synthetic", options }));
    }
    const provider = create({ apiKey: "synthetic" });
    for (const model of [
      undefined,
      null,
      [],
      { upstream: "synthetic", options: undefined },
      { upstream: "synthetic", defaults: undefined },
      { upstream: "synthetic", unknown: true },
      accessor("upstream"),
    ]) {
      reject("model shape", () => provider.model(`model-${next++}`, model));
    }
  }
  function rejectOptions(label, createProvider, options) {
    reject(`${label} provider`, () =>
      createProvider(gateway, `case-${next++}`, { apiKey: "synthetic", options }),
    );
    const provider = createProvider(gateway, `case-${next++}`, { apiKey: "synthetic" });
    reject(`${label} model`, () =>
      provider.model(`model-${next++}`, { upstream: "synthetic", options }),
    );
  }
  for (const [field, values] of [
    ["reasoningEffort", [undefined, null, "unknown", 1]],
    ["verbosity", [undefined, "unknown", false]],
    ["serviceTier", [undefined, "unknown", 1]],
    ["store", [undefined, 1, "false"]],
    ["parallelToolCalls", [undefined, 0, "true"]],
  ]) {
    for (const value of values) rejectOptions(field, createOpenAI, { [field]: value });
    rejectOptions(`${field} accessor`, createOpenAI, accessor(field));
  }
  for (const options of [
    { thinking: undefined },
    { thinking: "auto" },
    { thinking: { type: "enabled" } },
    { reasoningEffort: "unknown" },
    { reasoningEffort: undefined },
    accessor("thinking"),
  ]) {
    rejectOptions("deepseek option", createDeepSeek, options);
  }
  for (const provider of [
    undefined,
    null,
    [],
    { unknown: true },
    accessor("order"),
    { order: undefined },
    { order: "synthetic" },
    { only: [1] },
    { ignore: [undefined] },
    { allowFallbacks: undefined },
    { allowFallbacks: "false" },
    { requireParameters: 1 },
    { zdr: "true" },
    { dataCollection: "unknown" },
    { sort: "unknown" },
  ]) {
    rejectOptions("openrouter provider", createOpenRouter, { provider });
  }
  const arrayAccessor = Object.defineProperty(["synthetic"], "0", {
    enumerable: true,
    get() {
      touched++;
      return "synthetic-secret";
    },
  });
  rejectOptions("provider array accessor", createOpenRouter, {
    provider: { order: arrayAccessor },
  });
  for (const reasoning of [
    undefined,
    null,
    [],
    { unknown: true },
    accessor("effort"),
    { effort: undefined },
    { effort: "unknown" },
    { maxTokens: undefined },
    { maxTokens: 0 },
    { maxTokens: -1 },
    { maxTokens: 1.5 },
    { maxTokens: Number.MAX_SAFE_INTEGER + 1 },
    { maxTokens: Infinity },
    { maxTokens: "32" },
    { effort: "high", maxTokens: 32 },
    { enabled: 1 },
    { exclude: "false" },
  ]) {
    rejectOptions("openrouter reasoning", createOpenRouter, { reasoning });
  }
  for (const [createProvider, defaults, options] of [
    [createOpenAI, { reasoning_effort: "high" }, { reasoningEffort: "high" }],
    [createOpenAI, { verbosity: "high" }, { verbosity: "high" }],
    [createOpenAI, { service_tier: "auto" }, { serviceTier: "auto" }],
    [createOpenAI, { store: false }, { store: false }],
    [createOpenAI, { parallel_tool_calls: true }, { parallelToolCalls: true }],
    [createDeepSeek, { thinking: { type: "enabled" } }, { thinking: "enabled" }],
    [createDeepSeek, { reasoning_effort: "low" }, { reasoningEffort: "low" }],
    [createOpenRouter, { provider: { sort: "price" } }, { provider: { sort: "latency" } }],
    [createOpenRouter, { reasoning: { effort: "low" } }, { reasoning: { effort: "low" } }],
  ]) {
    reject("defaults conflict provider", () =>
      createProvider(gateway, `case-${next++}`, { apiKey: "synthetic", defaults, options }),
    );
    const provider = createProvider(gateway, `case-${next++}`, { apiKey: "synthetic" });
    reject("defaults conflict model", () =>
      provider.model(`model-${next++}`, { upstream: "synthetic", defaults, options }),
    );
  }
  for (const [createProvider, config, headers] of [
    [createOpenAI, { organization: "synthetic" }, { "oPeNaI-oRgAnIzAtIoN": "synthetic-secret" }],
    [createOpenAI, { project: "synthetic" }, { "openai-project": "synthetic-secret" }],
    [
      createOpenRouter,
      { appUrl: "https://synthetic.invalid" },
      { "http-referer": "synthetic-secret" },
    ],
    [createOpenRouter, { appTitle: "synthetic" }, { "x-openrouter-title": "synthetic-secret" }],
  ]) {
    reject("dedicated header conflict", () =>
      createProvider(gateway, `case-${next++}`, { apiKey: "synthetic", ...config, headers }),
    );
  }
  gateway.onRequest((ctx) => {
    ctx.state.failures = failures;
    ctx.state.touched = touched;
    ctx.state.checked = checked;
  });
});
