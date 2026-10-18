import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";
import { capCompletionTokens } from "limen:policies";

const scopes = [];
let closedStep;
let frozen = true;
let closedEarly = 0;
export default defineGateway((gateway) => {
  for (const [index, createProvider] of [
    createOpenAI,
    createDeepSeek,
    createOpenRouter,
  ].entries()) {
    const provider = createProvider(gateway, `provider-${index}`, { apiKey: "synthetic-key" });
    const model = provider.model(`model-${index}`, { upstream: "synthetic-model" });
    frozen &&= Object.isFrozen(provider) && Object.isFrozen(model);
    const native = capCompletionTokens({ name: "cap", max: 16 });
    provider.use(native);
    model.use(native);
    provider.step("provider-script", (step) => {
      closedStep = step;
      step.beforeSend((ctx) => {
        ctx.state.trace.push("provider");
        ctx.body.provider = true;
      });
    });
    try {
      closedStep.onResponse(() => {});
    } catch (error) {
      if (error instanceof TypeError) closedEarly++;
    }
    model.step("model-script", (step) => {
      step.beforeSend(async (ctx) => {
        await Promise.resolve();
        ctx.state.trace.push("model");
        ctx.body.model = "synthetic-overridden";
      });
    });
    scopes.push([provider, model]);
  }
  gateway.onRequest((ctx) => {
    let sealed = 0;
    for (const [provider, model] of scopes) {
      for (const late of [
        () => provider.model("late", { upstream: "synthetic" }),
        () => provider.use(capCompletionTokens({ name: "late", max: 1 })),
        () => provider.step("late", () => {}),
        () => model.use(capCompletionTokens({ name: "late", max: 1 })),
        () => model.step("late", () => {}),
      ]) {
        try {
          late();
        } catch (error) {
          if (error instanceof TypeError) sealed++;
        }
      }
    }
    ctx.state.frozen = frozen;
    ctx.state.closedEarly = closedEarly;
    ctx.state.sealed = sealed;
  });
});
