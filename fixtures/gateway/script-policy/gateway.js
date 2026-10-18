import { defineGateway } from "limen:gateway";
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

export default defineGateway((gateway) => {
  gateway.onRequest(async function (ctx) {
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
  });
  gateway.onError(function (ctx) {
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
  });
  gateway.onFinish(function (ctx) {
    checkClient(ctx);
    if (errors !== (ctx.outcome === "success" ? 0 : 1)) throw new Error("wrong observation count");
  });
  const provider0 = gateway.provider("demo", {
    endpoint: "https://example.invalid/v1/chat/completions",
    auth: { bearer: "synthetic-upstream-secret" },
  });
  provider0.use({
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
  });
  const model0 = provider0.model("chat", { upstream: "demo-model" });
  model0.use({
    name: "model-policy",
    beforeSend(ctx) {
      checkClient(ctx);
      if (ctx.state.mode === "model-reject") {
        ctx.state.rejectedBy = "model";
        return ctx.reject(403);
      }
      ctx.state.model = true;
    },
  });
});
