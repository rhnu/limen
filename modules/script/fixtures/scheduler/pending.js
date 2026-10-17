let started = 0;
export default {
  onRequest(ctx) {
    if (ctx.state.pending) {
      started++;
      return new Promise(() => {});
    }
    ctx.state.done = true;
  },
  onError(ctx) { ctx.state.started = started; }
};
