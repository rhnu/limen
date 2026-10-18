import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";

export default defineGateway((gateway) => {
  for (const effort of ["none", "minimal", "low", "medium", "high", "xhigh", "max"]) {
    for (const [id, createProvider, options] of [
      ["openai", createOpenAI, { reasoningEffort: effort }],
      ["deepseek", createDeepSeek, { reasoningEffort: effort }],
      ["openrouter", createOpenRouter, { reasoning: { effort } }],
    ]) {
      createProvider(gateway, `${id}-${effort}`, { apiKey: "synthetic", options }).model(
        `${id}-${effort}`,
        { upstream: "synthetic", options },
      );
    }
  }
  for (const serviceTier of ["auto", "default", "flex", "scale", "priority", "fast"]) {
    createOpenAI(gateway, `tier-${serviceTier}`, { apiKey: "synthetic", options: { serviceTier } });
  }
  for (const verbosity of ["low", "medium", "high"]) {
    createOpenAI(gateway, `verbosity-${verbosity}`, {
      apiKey: "synthetic",
      options: { verbosity },
    });
  }
  for (const sort of ["price", "throughput", "latency"]) {
    createOpenRouter(gateway, `sort-${sort}`, {
      apiKey: "synthetic",
      options: { provider: { sort } },
    });
  }
  createOpenRouter(gateway, "maximum", {
    apiKey: "synthetic",
    options: { reasoning: { maxTokens: Number.MAX_SAFE_INTEGER } },
  });
  createOpenAI(gateway, "raw-headers", {
    apiKey: "synthetic",
    headers: { "OpenAI-Organization": "synthetic-org", "OpenAI-Project": "synthetic-project" },
    defaults: { vendor_extension: { enabled: true } },
  });
  createOpenRouter(gateway, "raw-attribution", {
    apiKey: "synthetic",
    endpoint: "https://synthetic.invalid/router/chat",
    headers: { "HTTP-Referer": "https://synthetic.invalid", "X-OpenRouter-Title": "Synthetic" },
  });
  createDeepSeek(gateway, "custom-deepseek", {
    apiKey: "synthetic",
    endpoint: "https://synthetic.invalid/deepseek/chat",
  });
  createDeepSeek(gateway, "duplicate-raw-headers", {
    apiKey: "synthetic",
    headers: { "X-Synthetic": "first", "x-synthetic": "second" },
  });
});
