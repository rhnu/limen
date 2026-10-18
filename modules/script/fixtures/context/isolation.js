let release;
let started = false;
export default {
  async onRequest(ctx) {
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
  },
  onFinish() {
    release();
  },
  onError(ctx) {
    ctx.state.started = started;
  },
  providers: {
    p: {
      pipeline: [
        {
          name: "provider",
          async beforeSend(ctx) {
            await Promise.resolve();
            ctx.state.order.push("provider");
            ctx.headers.append("x-step", "provider");
          },
        },
      ],
    },
  },
  models: {
    m: {
      pipeline: [
        {
          name: "model",
          async beforeSend(ctx) {
            await Promise.resolve();
            ctx.state.order.push("model");
            ctx.headers.append("x-step", "model");
          },
        },
      ],
    },
  },
};
