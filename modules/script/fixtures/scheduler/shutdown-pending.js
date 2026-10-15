let started = false;
export default {
  onRequest(ctx) {
    return new Promise(() => { started = true; ctx.pending = true; });
  },
  onError(ctx) { ctx.started = started; }
};
