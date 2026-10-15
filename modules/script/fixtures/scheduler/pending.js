let started = 0;
export default {
  onRequest(ctx) {
    if (ctx.pending) {
      started++;
      return new Promise(() => {});
    }
    ctx.done = true;
  },
  onError(ctx) { ctx.started = started; }
};
