import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  const p = gateway.provider("p", { endpoint: "https://fictional.invalid/chat" });
  p.use({
    name: "provider",
    beforeSend(ctx) {
      ctx.state.order.push("provider");
    },
  });
  const m = p.model("m", { upstream: "synthetic" });
  m.use({
    name: "model",
    async beforeSend(ctx) {
      await Promise.resolve();
      ctx.state.order.push("model");
    },
  });
  gateway.onRequest((ctx) => {
    ctx.state.sync = true;
  });
  gateway.onFinish(async (ctx) => {
    await Promise.resolve();
    ctx.state.async = true;
  });
});
