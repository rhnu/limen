import { defineGateway } from "limen:gateway";
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

export default defineGateway((gateway) => {
  gateway.onRequest(deep);
  const provider0 = gateway.provider("p", { endpoint: "https://example.invalid" });
  provider0.use({ name: "deep", onStreamEvent: deep });
  const model0 = provider0.model("m", { upstream: "m" });
});
