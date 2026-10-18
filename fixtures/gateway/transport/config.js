import { endpoint } from "../endpoint.js";

export default {
  onRequest(ctx) {
    if (ctx.body.rewrite) ctx.body.stream = false;
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
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
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
