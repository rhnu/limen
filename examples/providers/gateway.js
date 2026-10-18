import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";
import { capCompletionTokens } from "limen:policies";

// Synthetic credentials, model names and endpoint overrides keep this example offline.
// Omit endpoint and supply your own key/model to use each factory's official endpoint.
export default defineGateway((gateway) => {
  const openai = createOpenAI(gateway, "openai", {
    apiKey: "example-openai-token",
    endpoint: "https://openai.example.invalid/v1/chat/completions",
    organization: "example-org",
    project: "example-project",
    defaults: { temperature: 0.6 },
    options: { reasoningEffort: "low", verbosity: "medium", store: false },
  });
  const chat = openai.model("openai-chat", {
    upstream: "synthetic-openai",
    defaults: { max_completion_tokens: 32 },
    options: { reasoningEffort: "high", verbosity: "low", parallelToolCalls: false },
  });
  chat.use(capCompletionTokens({ name: "cap-output", max: 64 }));

  const deepseek = createDeepSeek(gateway, "deepseek", {
    apiKey: "example-deepseek-token",
    endpoint: "https://deepseek.example.invalid/chat/completions",
    options: { thinking: "enabled", reasoningEffort: "low" },
  });
  deepseek.model("deepseek-chat", {
    upstream: "synthetic-deepseek",
    defaults: { max_tokens: 32 },
    options: { reasoningEffort: "high" },
  });

  const router = createOpenRouter(gateway, "openrouter", {
    apiKey: "example-openrouter-token",
    endpoint: "https://openrouter.example.invalid/api/v1/chat/completions",
    appUrl: "https://example.invalid/limen",
    appTitle: "Limen example",
    options: {
      provider: { sort: "latency", allowFallbacks: false, dataCollection: "deny" },
    },
  });
  router.model("router-chat", {
    upstream: "synthetic/router",
    options: { reasoning: { effort: "high", exclude: false } },
  });
  // A model's nested object replaces the provider's entire object (shallow defaults).
  router.model("router-price", {
    upstream: "synthetic/router",
    options: { provider: { sort: "price" } },
  });
});
