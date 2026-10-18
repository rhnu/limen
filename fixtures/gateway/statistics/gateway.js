import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

let failedSnapshot;

function require(condition) {
  if (!condition) throw new Error("synthetic-statistics-assertion");
}

function assertFrozen(value) {
  if (value === null || typeof value !== "object") return;
  require(Object.isFrozen(value));
  for (const nested of Object.values(value)) assertFrozen(nested);
}

function checkStats(ctx) {
  const { usage, timings } = ctx.stats;
  assertFrozen(ctx.stats);
  require(typeof timings.requestMs === "number");
  require(!("onError" in timings.hooks) && !("onFinish" in timings.hooks));
  for (const mutate of [
    () => {
      usage.inputTokens = 999;
    },
    () => {
      timings.requestMs = 999;
    },
    () => {
      timings.hooks.onRequest.queue.samples = 999;
    },
    () => {
      delete usage.quality;
    },
    () => {
      ctx.stats.usage = {};
    },
  ]) {
    let rejected = false;
    try {
      mutate();
    } catch {
      rejected = true;
    }
    require(rejected);
  }
  return { usage, timings };
}

function rewrittenUsage() {
  return { prompt_tokens: 90, completion_tokens: 9, total_tokens: 99 };
}

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    ctx.state.mode = ctx.body.mode || "delete";
  });
  gateway.onError(function (ctx) {
    failedSnapshot = checkStats(ctx);
  });
  gateway.onFinish(function (ctx) {
    const current = checkStats(ctx);
    if (failedSnapshot) {
      require(current.usage !== failedSnapshot.usage);
      require(current.timings !== failedSnapshot.timings);
      require(JSON.stringify(current) === JSON.stringify(failedSnapshot));
    }
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "statistics-transform",
    beforeSend() {},
    afterReceive() {},
    onResponse(ctx) {
      if (ctx.state.mode === "rewrite") ctx.body.usage = rewrittenUsage();
      else delete ctx.body.usage;
    },
    onUpstreamEvent(ctx) {
      if (ctx.event.usage && ctx.state.mode === "rewrite") {
        ctx.event.usage = rewrittenUsage();
      } else {
        delete ctx.event.usage;
      }
    },
    onStreamEvent(ctx) {
      delete ctx.event.usage;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
