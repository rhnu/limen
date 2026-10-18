import { endpoint } from "../endpoint.js";

function record(ctx, name) {
  if (ctx.observe.record(name) !== undefined) {
    throw new Error("synthetic-observation-return");
  }
}

export default {
  onRequest(ctx) {
    ctx.state.mode = ctx.body.mode || "normal";
    record(ctx, "request");
    record(ctx, "request");
  },
  onError(ctx) {
    record(ctx, "error");
    if (["observer_error", "observer_budget"].includes(ctx.state.mode)) {
      record(ctx, "extra");
      record(ctx, "overflow");
    }
    if (ctx.state.mode === "observer_error") throw new Error("synthetic-private-value");
    if (ctx.state.mode === "observer_budget") return new Promise(() => {});
  },
  onFinish(ctx) {
    record(ctx, "finish");
    if (ctx.state.mode === "observer_finish") {
      record(ctx, "extra");
      record(ctx, "overflow");
      throw new Error("synthetic-private-value");
    }
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "provider",
          beforeSend(ctx) {
            ctx.state.committed = "provider";
            record(ctx, "provider");
          },
          afterReceive(ctx) {
            record(ctx, "response");
            if (ctx.state.mode === "status") {
              record(ctx, "extra");
              record(ctx, "overflow");
              ctx.state.discarded = true;
              ctx.status = 204;
            }
          },
          onResponse(ctx) {
            record(ctx, "response");
          },
          onUpstreamEvent(ctx) {
            record(ctx, "upstream");
            if (ctx.state.mode === "overflow") {
              record(ctx, "upstream");
              record(ctx, "extra");
              record(ctx, "overflow");
            }
          },
          onStreamEvent(ctx) {
            record(ctx, "stream");
          },
        },
      ],
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "real-model",
      pipeline: [
        {
          name: "model",
          async beforeSend(ctx) {
            record(ctx, "model");
            ctx.state.committed = "model";
            const mode = ctx.state.mode;
            if (
              [
                "throw",
                "observer_error",
                "observer_budget",
                "promise",
                "cycle",
                "url",
                "header",
              ].includes(mode)
            ) {
              record(ctx, "extra");
              record(ctx, "overflow");
            }
            if (["throw", "observer_error", "observer_budget"].includes(mode)) {
              throw new Error("synthetic-private-value");
            }
            if (mode === "promise") await Promise.reject(new Error("synthetic-private-value"));
            if (mode === "cycle") ctx.body.cycle = ctx.body;
            if (mode === "url") ctx.url = "file:///synthetic-private-value";
            if (mode === "header") ctx.headers.set("content-type", "synthetic-private-value");
            if (mode === "reject") ctx.reject(403);
          },
          onUpstreamEvent(ctx) {
            if (ctx.state.mode === "overflow") return;
            record(ctx, "model");
            if (ctx.state.mode === "event") {
              record(ctx, "extra");
              record(ctx, "overflow");
              ctx.state.discarded = true;
              ctx.event.choices[0].delta.content = 7;
            }
          },
        },
      ],
    },
  },
};
