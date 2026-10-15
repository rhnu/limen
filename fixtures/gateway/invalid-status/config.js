import { endpoint } from "../endpoint.js";

export default {
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
          name: "invalid-status",
          afterReceive(ctx) {
            ctx.status = 200;
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
