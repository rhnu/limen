import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";

let cancelled = 0;
let finishes = 0;
let waiting = 0;

export function configurePending(gateway, failCancelledFinish = false) {
  gateway.onRequest(function (ctx) {
    if (ctx.body.wait) {
      waiting++;
      return new Promise(() => {});
    }
  });
  gateway.onFinish(function (ctx) {
    finishes++;
    if (ctx.outcome === "cancelled") {
      cancelled++;
      if (failCancelledFinish) throw new Error("observer failure");
    }
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "report",
    onResponse(ctx) {
      ctx.body.cancelled = cancelled;
      ctx.body.finishes = finishes;
      ctx.body.waiting = waiting;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
}

export default defineGateway((gateway) => {
  configurePending(gateway);
});
