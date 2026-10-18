import { endpoint } from "./fixture-data.js";

export default {
  providers: {
    p: {
      endpoint: "https://unused.invalid/chat/completions",
      auth: { bearer: "synthetic-upstream-secret" },
      pipeline: [
        {
          name: "dynamic-target",
          beforeSend(ctx) {
            ctx.url = endpoint;
          },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
