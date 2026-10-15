import { endpoint } from "./fixture-data.js";

let cancelled = 0;
let finishes = 0;
let waiting = 0;

export default {
  onRequest(ctx) {
    if (ctx.body.wait) {
      waiting++;
      return new Promise(() => {});
    }
  },
  onFinish(ctx) {
    finishes++;
    if (ctx.outcome === "cancelled") cancelled++;
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "report",
          onResponse(ctx) {
            ctx.body.cancelled = cancelled;
            ctx.body.finishes = finishes;
            ctx.body.waiting = waiting;
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
