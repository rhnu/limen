export default {
  onRequest(ctx) {
    ctx.state.sync = true;
  },
  async onFinish(ctx) {
    await Promise.resolve();
    ctx.state.async = true;
  },
  providers: {
    p: {
      pipeline: [
        {
          name: "provider",
          beforeSend(ctx) {
            ctx.state.order.push("provider");
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
          },
        },
      ],
    },
  },
};
