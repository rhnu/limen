import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";

let waiting = 0;
let cancelled = 0;

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    if (ctx.body.wait) {
      waiting++;
      ctx.observe.record("pending_private");
      return new Promise(() => {});
    }
    ctx.observe.record("accepted");
    if (ctx.body.cleanupWait) {
      ctx.state.cleanupWait = true;
      ctx.reject(403);
    }
  });
  gateway.onError(function (ctx) {
    if (ctx.state.cleanupWait) {
      ctx.observe.record("cleanup_private");
      return new Promise(() => {});
    }
  });
  gateway.onFinish(function (ctx) {
    if (ctx.outcome === "cancelled") cancelled++;
    ctx.observe.record("finished");
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "report",
    onResponse(ctx) {
      ctx.body.waiting = waiting;
      ctx.body.cancelled = cancelled;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
