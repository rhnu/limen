import { endpoint } from "../endpoint.js";

let errors = 0;
let finishes = 0;

export default {
  onRequest(ctx) {
    ctx.state.mode = ctx.body.mode || "normal";
  },
  onError(ctx) {
    errors++;
    if (ctx.state.mode === "error-wait") return new Promise(() => {});
    throw new Error("synthetic-body-secret");
  },
  onFinish(ctx) {
    finishes++;
    if (ctx.state.mode === "finish-wait") return new Promise(() => {});
    if (ctx.state.mode === "bad") throw new Error("synthetic-body-secret");
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "probe",
          beforeSend(ctx) {
            if (ctx.state.mode === "bad" || ctx.state.mode === "error-wait") {
              throw new Error("synthetic-body-secret");
            }
          },
          onResponse(ctx) {
            ctx.body.counts = [errors, finishes];
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
