export default () => ({
  onRequest(ctx) {
    if (ctx.state.kind === 0) ctx.state.bad = undefined;
    if (ctx.state.kind === 1) ctx.state.bad = [,];
    if (ctx.state.kind === 2) ctx.state.bad = 9007199254740992;
    if (ctx.state.kind === 3) ctx.state.bad = ctx;
  },
});
