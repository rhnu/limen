import { defineGateway } from "limen:gateway";
import { capCompletionTokens } from "limen:policies";

let builds = 0;
let closedStep;
let abortedGateway, abortedProvider, abortedModel;
let aborted = 0;
try {
  defineGateway((gateway) => {
    abortedGateway = gateway;
    abortedProvider = gateway.provider("failed", { endpoint: "https://fictional.invalid/chat" });
    abortedModel = abortedProvider.model("failed", { upstream: "synthetic" });
    throw Error("abort setup");
  })();
} catch {
  aborted++;
}
for (const operation of [
  () => abortedGateway.onRequest(() => {}),
  () => abortedProvider.model("late", { upstream: "synthetic" }),
  () => abortedModel.step("late", () => {}),
]) {
  try {
    operation();
  } catch {
    aborted++;
  }
}
export default defineGateway((gateway) => {
  builds++;
  const provider = gateway.provider("__proto__", { endpoint: "https://fictional.invalid/chat" });
  const model = provider.model("constructor", { upstream: "synthetic" });
  const other = provider.model("__proto__", { upstream: "other" });
  const native = capCompletionTokens({ name: "cap", max: 16 });
  provider.use(native);
  model.use(native);
  other.use(native);
  provider.step("script", (step) => {
    closedStep = step;
    step.beforeSend((ctx) => {
      ctx.state.phase = "before";
    });
    step.onResponse((ctx) => {
      ctx.state.phase = "response";
    });
  });
  let rejected = 0;
  try {
    closedStep.onContent(() => {});
  } catch {
    rejected++;
  }
  let failedScope;
  try {
    provider.step("rolled-back", (step) => {
      failedScope = step;
      step.beforeSend(() => {});
      throw Error("setup failed");
    });
  } catch {
    rejected++;
  }
  try {
    failedScope.onContent(() => {});
  } catch {
    rejected++;
  }
  provider.step("rolled-back", (step) => {
    step.onContent(() => {});
  });
  gateway.onRequest((ctx) => {
    const late = [
      () => gateway.provider("late", { endpoint: "https://fictional.invalid/chat" }),
      () => provider.model("late", { upstream: "late" }),
      () => provider.use(native),
      () => model.use(native),
      () => other.step("late", () => {}),
      () => gateway.onFinish(() => {}),
      () => closedStep.beforeSend(() => {}),
    ];
    let sealed = 0;
    for (const operation of late) {
      try {
        operation();
      } catch {
        sealed++;
      }
    }
    ctx.state.builds = builds;
    ctx.state.rejected = rejected;
    ctx.state.sealed = sealed;
    ctx.state.aborted = aborted;
  });
});
