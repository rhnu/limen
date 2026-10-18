import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    ctx.state.mode = ctx.body.mode || "normal";
    ctx.state.original = "retained";
  });
  gateway.onError(function (ctx) {
    // Observation writes are rejected and cannot repair the terminal result.
    ctx.state.original = "must-not-commit";
    throw new Error("synthetic-body-secret");
  });
  gateway.onFinish(function (ctx) {
    if (ctx.state.mode === "readonly") ctx.state.original = "must-not-commit";
    if (ctx.state.mode === "throw" || ctx.state.mode === "bad") {
      throw new Error("synthetic-body-secret");
    }
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "failure",
    beforeSend(ctx) {
      if (ctx.state.mode === "bad") throw new Error("synthetic-body-secret");
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
