import { endpoint } from "./fixture-data.js";

let waiting = 0;
let cancelled = 0;

export default {
  onRequest(ctx) {
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
  },
  onError(ctx) {
    if (ctx.state.cleanupWait) {
      ctx.observe.record("cleanup_private");
      return new Promise(() => {});
    }
  },
  onFinish(ctx) {
    if (ctx.outcome === "cancelled") cancelled++;
    ctx.observe.record("finished");
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "report",
          onResponse(ctx) {
            ctx.body.waiting = waiting;
            ctx.body.cancelled = cancelled;
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
