function deep(ctx) {
  if (ctx.state.input) {
    let value = (ctx.meta.phase === "onStreamEvent" ? ctx.event : ctx.body).nested,
      depth = 0;
    while (Array.isArray(value)) {
      value = value[0];
      depth++;
    }
    ctx.state.inputArrays = depth;
    if (ctx.meta.phase === "onStreamEvent") ctx.event = {};
    else ctx.body = {};
    return;
  }
  let nested = null;
  for (let i = 0; i < ctx.state.arrays; i++) nested = [nested];
  if (ctx.meta.phase === "onStreamEvent") ctx.event = { nested };
  else ctx.body = { nested };
}
export default {
  providers: {
    p: { endpoint: "https://example.invalid", pipeline: [{ name: "deep", onStreamEvent: deep }] },
  },
  models: { m: { provider: "p", upstream: "m" } },
  onRequest: deep,
};
