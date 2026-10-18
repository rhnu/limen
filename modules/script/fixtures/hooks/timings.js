export default {
  onRequest(ctx) {
    if (ctx.state.mode === "throw") throw new Error("synthetic");
    if (ctx.state.mode === "invalid") ctx.state.value = undefined;
    if (ctx.state.mode === "contract") return 1;
    ctx.state.done = true;
  },
  async onFinish(ctx) {
    await Promise.resolve();
    if (ctx.state.mode === "throw") throw new Error("synthetic");
    if (ctx.state.mode === "invalid") ctx.state.value = undefined;
    if (ctx.state.mode === "contract") return 1;
    ctx.state.done = true;
  },
};
