import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";

export const cases = [
  defineGateway((gateway) => {
    createOpenAI(gateway, "synthetic-secret-provider", {
      apiKey: "synthetic-secret",
      options: { reasoningEffort: "synthetic-secret" },
    });
  }),
  defineGateway((gateway) => {
    createDeepSeek(gateway, "synthetic-secret-provider", {
      apiKey: "synthetic-secret",
      options: { thinking: undefined },
    });
  }),
  defineGateway((gateway) => {
    createOpenRouter(gateway, "synthetic-secret-provider", {
      apiKey: "synthetic-secret",
      options: { reasoning: { effort: "high", maxTokens: 1 } },
    });
  }),
  defineGateway((gateway) => {
    createOpenAI(gateway, "synthetic-secret-provider", { apiKey: "synthetic-secret" }).model(
      "synthetic-secret-model",
      { upstream: "synthetic-secret", defaults: { store: false }, options: { store: true } },
    );
  }),
];
