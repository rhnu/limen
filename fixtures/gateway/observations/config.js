import { endpoint } from "../endpoint.js";

export default {
  onRequest(ctx) {
    ctx.state.mode = ctx.body.mode || "normal";
    ctx.state.original = "retained";
  },
  onError(ctx) {
    // Observation writes are rejected and cannot repair the terminal result.
    ctx.state.original = "must-not-commit";
    throw new Error("synthetic-body-secret");
  },
  onFinish(ctx) {
    if (ctx.state.mode === "readonly") ctx.state.original = "must-not-commit";
    if (ctx.state.mode === "throw" || ctx.state.mode === "bad") {
      throw new Error("synthetic-body-secret");
    }
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "failure",
          beforeSend(ctx) {
            if (ctx.state.mode === "bad") throw new Error("synthetic-body-secret");
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
