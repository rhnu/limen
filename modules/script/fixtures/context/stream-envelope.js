import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    if (ctx.meta.operation === "field") ctx.sse.event = "changed";
    else if (ctx.meta.operation === "replace") ctx.sse = null;
    else {
      ctx.state.envelope = ctx.sse;
      ctx.event = { annotations: [] };
    }
  });
});
