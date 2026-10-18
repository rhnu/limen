import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";

export default defineGateway((gateway) => {
  const openai = createOpenAI(gateway, "openai", {
    apiKey: "synthetic-openai",
    organization: "synthetic-org",
    project: "synthetic-project",
    headers: { "X-Synthetic": "openai" },
    defaults: { temperature: 0.5 },
    options: {
      reasoningEffort: "xhigh",
      verbosity: "high",
      serviceTier: "priority",
      store: false,
      parallelToolCalls: true,
    },
  });
  openai.model("openai-model", {
    upstream: "synthetic-chat",
    defaults: { max_completion_tokens: 128 },
    options: {
      reasoningEffort: "minimal",
      verbosity: "low",
      serviceTier: "fast",
      store: true,
      parallelToolCalls: false,
    },
  });
  const deepseek = createDeepSeek(gateway, "deepseek", {
    apiKey: "synthetic-deepseek",
    options: { thinking: "enabled", reasoningEffort: "max" },
  });
  deepseek.model("deepseek-model", {
    upstream: "synthetic-reasoner",
    options: { thinking: "disabled", reasoningEffort: "none" },
  });
  const providerOptions = {
    order: ["synthetic-first", "synthetic-second"],
    only: ["synthetic-only"],
    ignore: ["synthetic-ignore"],
    allowFallbacks: false,
    requireParameters: true,
    dataCollection: "deny",
    sort: "latency",
    zdr: true,
  };
  const reasoningOptions = { maxTokens: 32, exclude: false, enabled: true };
  const modelOrder = ["synthetic-model"];
  const modelOptions = {
    provider: { order: modelOrder, dataCollection: "allow", sort: "throughput" },
    reasoning: { effort: "high", exclude: true, enabled: false },
  };
  const openrouter = createOpenRouter(gateway, "openrouter", {
    apiKey: "synthetic-openrouter",
    appUrl: "https://synthetic.invalid/app",
    appTitle: "Synthetic application",
    headers: { "X-Synthetic": "openrouter" },
    options: { provider: providerOptions, reasoning: reasoningOptions },
  });
  openrouter.model("openrouter-model", {
    upstream: "synthetic/vendor-model",
    defaults: { temperature: 0.25 },
    options: modelOptions,
  });
  providerOptions.order[0] = "changed";
  providerOptions.order.push("changed");
  providerOptions.only[0] = "changed";
  providerOptions.ignore[0] = "changed";
  providerOptions.allowFallbacks = true;
  reasoningOptions.maxTokens = 1;
  modelOrder[0] = "changed";
  modelOptions.provider.sort = "price";
  modelOptions.reasoning.effort = "none";
  createOpenAI(gateway, "custom", {
    apiKey: "synthetic-custom",
    endpoint: "https://synthetic.invalid/full/chat/completions?version=synthetic",
  }).model("custom-model", { upstream: "synthetic-custom" });
});
