export default {
  onRequest(ctx) {
    if (ctx.kind === 0) ctx.bad = undefined;
    if (ctx.kind === 1) ctx.bad = [,];
    if (ctx.kind === 2) ctx.bad = 9007199254740992;
    if (ctx.kind === 3) ctx.bad = ctx;
  }
};
