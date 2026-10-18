import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

let errors = 0;
let finishes = 0;

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    ctx.state.mode = ctx.body.mode || "normal";
  });
  gateway.onError(function (ctx) {
    errors++;
    if (ctx.state.mode === "error-wait") return new Promise(() => {});
    throw new Error("synthetic-body-secret");
  });
  gateway.onFinish(function (ctx) {
    finishes++;
    if (ctx.state.mode === "finish-wait") return new Promise(() => {});
    if (ctx.state.mode === "bad") throw new Error("synthetic-body-secret");
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "probe",
    beforeSend(ctx) {
      if (ctx.state.mode === "bad" || ctx.state.mode === "error-wait") {
        throw new Error("synthetic-body-secret");
      }
    },
    onResponse(ctx) {
      ctx.body.counts = [errors, finishes];
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
