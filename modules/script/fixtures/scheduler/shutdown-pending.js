let started = false;
export default {
  onRequest(ctx) {
    return new Promise(() => {
      started = true;
      ctx.state.pending = true;
    });
  },
  onError(ctx) {
    ctx.state.started = started;
  },
};
