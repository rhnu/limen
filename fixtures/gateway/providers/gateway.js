import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";
import { capCompletionTokens } from "limen:policies";

/** @param {import("limen:gateway").ProviderScope} provider @param {string} alias */
function install(provider, alias) {
  provider.step("provider-order", (step) => {
    step.beforeSend((ctx) => {
      ctx.body.order = "provider";
    });
  });
  provider.use(capCompletionTokens({ name: "provider-cap", max: 64 }));
  const model = provider.model(alias, { upstream: "synthetic-model" });
  model.step("model-order", (step) => {
    step.beforeSend((ctx) => {
      ctx.body.order = [ctx.body.order ?? null, "model"];
    });
  });
  model.use(capCompletionTokens({ name: "model-cap", max: 16 }));
}

export default defineGateway((gateway) => {
  install(createOpenAI(gateway, "openai", { apiKey: "synthetic-openai" }), "openai");
  install(createDeepSeek(gateway, "deepseek", { apiKey: "synthetic-deepseek" }), "deepseek");
  install(createOpenRouter(gateway, "router", { apiKey: "synthetic-router" }), "router");
});
