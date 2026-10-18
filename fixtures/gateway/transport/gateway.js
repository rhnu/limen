import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    if (ctx.body.rewrite) ctx.body.stream = false;
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "transport-facts",
    beforeSend(ctx) {
      if (ctx.body.upstreamStream) ctx.body.stream = true;
    },
    afterReceive(ctx) {
      ctx.state.clientStream = ctx.meta.clientStream;
      ctx.state.upstreamStatus = ctx.meta.upstreamStatus;
      ctx.state.upstreamMedia = ctx.meta.upstreamContentType;
      // Changing the draft cannot select a parser or reclassify upstream failure.
      ctx.headers.set("X-Original-Media", ctx.meta.upstreamContentType);
      if (ctx.meta.upstreamStatus >= 300) {
        ctx.status = 409;
        ctx.headers.set("X-Mapped", "failure");
      }
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
