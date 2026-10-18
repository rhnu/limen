function checkClient(ctx) {
  const actual = {
    profiles: ctx.client.headers.getAll("X-Profile"),
    tags: ctx.client.headers.getAll("x-TAGS"),
  };
  if (
    ctx.client.headers.getAll("authorization").length ||
    ctx.client.headers.getAll("cookie").length ||
    ctx.client.headers.getAll("x-private").length
  )
    throw new Error("unexposed header visible");
  if (ctx.state.client && JSON.stringify(actual) !== JSON.stringify(ctx.state.client)) {
    throw new Error("client context changed between phases");
  }
  return actual;
}

let errors = 0;
export default {
  async onRequest(ctx) {
    ctx.state.client = checkClient(ctx);
    ctx.client.headers.getAll("x-profile").push("detached");
    ctx.state.mode = ctx.body.mode || "normal";
    if (ctx.body.mode === "request-default") return ctx.reject();
    if (ctx.body.mode === "request-async") {
      await Promise.resolve();
      return ctx.reject(403);
    }
    if (ctx.body.mode === "reject-throw") {
      ctx.state.uncommitted = true;
      ctx.reject(400);
      throw new Error("synthetic private script data");
    }
    if (ctx.body.mode === "reject-return") {
      ctx.state.uncommitted = true;
      ctx.reject(400);
      return {};
    }
    if (ctx.body.mode === "reject-invalid-body") {
      ctx.body.messages = [];
      return ctx.reject(422);
    }
    if (ctx.body.mode === "readonly") ctx.client.headers.set("x-profile", "changed");
  },
  providers: {
    demo: {
      endpoint: "https://example.invalid/v1/chat/completions",
      auth: { bearer: "synthetic-upstream-secret" },
      pipeline: [
        {
          name: "provider-policy",
          async beforeSend(ctx) {
            checkClient(ctx);
            if (ctx.state.mode === "provider-reject") {
              await Promise.resolve();
              ctx.state.rejectedBy = "provider";
              return ctx.reject(422);
            }
            if (ctx.state.mode === "reject-promise") {
              ctx.state.uncommitted = true;
              ctx.reject(403);
              await Promise.reject(new Error("synthetic private policy data"));
            }
            ctx.state.provider = true;
          },
          afterReceive(ctx) {
            checkClient(ctx);
          },
          onResponse(ctx) {
            checkClient(ctx);
          },
          onUpstreamEvent(ctx) {
            checkClient(ctx);
          },
          onStreamEvent(ctx) {
            checkClient(ctx);
          },
        },
      ],
    },
  },
  models: {
    chat: {
      provider: "demo",
      upstream: "demo-model",
      pipeline: [
        {
          name: "model-policy",
          beforeSend(ctx) {
            checkClient(ctx);
            if (ctx.state.mode === "model-reject") {
              ctx.state.rejectedBy = "model";
              return ctx.reject(403);
            }
            ctx.state.model = true;
          },
        },
      ],
    },
  },
  onError(ctx) {
    errors += 1;
    checkClient(ctx);
    if (ctx.error.code === "request_rejected") {
      const status =
        ctx.state.mode === "request-default"
          ? 400
          : ctx.state.mode === "provider-reject"
            ? 422
            : 403;
      if (
        ctx.outcome !== "failed" ||
        ctx.error.status !== status ||
        ctx.stats.outboundBytes !== 0
      ) {
        throw new Error("wrong rejection observation");
      }
      if (ctx.state.mode === "provider-reject" && ctx.error.step !== "provider-policy") {
        throw new Error("wrong rejecting step");
      }
    }
  },
  onFinish(ctx) {
    checkClient(ctx);
    if (errors !== (ctx.outcome === "success" ? 0 : 1)) throw new Error("wrong observation count");
  },
};
