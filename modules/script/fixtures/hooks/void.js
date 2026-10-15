export default {
  onRequest(ctx) { ctx.sync = true; },
  async onFinish(ctx) {
    await Promise.resolve();
    ctx.async = true;
  },
  providers: { p: { pipeline: [{
    name: 'provider',
    beforeSend(ctx) { ctx.order.push('provider'); }
  }] } },
  models: { m: { pipeline: [{
    name: 'model',
    async beforeSend(ctx) {
      await Promise.resolve();
      ctx.order.push('model');
    }
  }] } }
};
