import { defineGateway } from "limen:gateway";
let release;
let started = false;
export default defineGateway((gateway) => {
  const p = gateway.provider("p", { endpoint: "https://fictional.invalid/chat" });
  p.use({
    name: "provider",
    async beforeSend(ctx) {
      await Promise.resolve();
      ctx.state.order.push("provider");
      ctx.headers.append("x-step", "provider");
    },
  });
  const m = p.model("m", { upstream: "synthetic" });
  m.use({
    name: "model",
    async beforeSend(ctx) {
      await Promise.resolve();
      ctx.state.order.push("model");
      ctx.headers.append("x-step", "model");
    },
  });
  gateway.onRequest(async (ctx) => {
    ctx.state.order = ["request"];
    ctx.body.private = ctx.meta.requestId;
    if (ctx.meta.pending)
      await new Promise((resolve) => {
        started = true;
        release = resolve;
      });
    await Promise.resolve();
    ctx.state.order.push("completed");
    if (ctx.meta.failure === "reject") throw new Error("synthetic");
    if (ctx.meta.failure === "return") return {};
  });
  gateway.onFinish(() => {
    release();
  });
  gateway.onError((ctx) => {
    ctx.state.started = started;
  });
});
