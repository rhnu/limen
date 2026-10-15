import { endpoint } from "../endpoint.js";

export default {
  onRequest(ctx) {
    if (ctx.body.model === "incoming") ctx.body.model = "chat";
    if (ctx.body.enforce) ctx.body.top_p = 0.75;
  },
  providers: {
    primary: {
      endpoint,
      auth: { bearer: "synthetic-upstream-secret" },
      headers: { "X-Policy": "static", "X-Remove": "static" },
      defaults: {
        temperature: 0.2,
        top_p: 0.9,
        max_completion_tokens: 128,
        reasoning: { effort: "low", summary: "auto" },
        stop: ["provider", "shared"],
      },
      pipeline: [
        {
          name: "compatibility",
          beforeSend(ctx) {
            delete ctx.body.store;
            ctx.headers.delete("x-remove");
            if (ctx.body.enforce) {
              ctx.body.temperature = 0.6;
              ctx.body.max_completion_tokens += 128;
              ctx.body.model = "decoy";
              ctx.headers.set("x-policy", "provider");
            }
          },
        },
        {
          name: "provider-policy",
          async beforeSend(ctx) {
            await Promise.resolve();
            if (ctx.body.enforce) {
              ctx.body.temperature = 0.5;
              ctx.headers.set("X-Policy", "provider-final");
            }
          },
        },
      ],
    },
    decoy: { endpoint, headers: { "X-Wrong-Route": "yes" } },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "real-model",
      defaults: {
        temperature: 0.4,
        max_completion_tokens: 256,
        reasoning: { effort: "medium" },
        stop: ["model"],
      },
      pipeline: [
        {
          name: "model-policy",
          async beforeSend(ctx) {
            await Promise.resolve();
            if (ctx.body.enforce) {
              ctx.body.temperature = 0.3;
              ctx.body.max_completion_tokens *= 2;
              ctx.headers.set("X-Policy", "model");
              ctx.headers.delete("AUTHORIZATION");
              ctx.url = ctx.url.replace("/custom/completion", "/model/completion");
            }
          },
        },
        {
          name: "final-policy",
          beforeSend(ctx) {
            if (ctx.body.enforce) {
              ctx.body.temperature = 0.1;
              ctx.headers.set("x-policy", "model-final");
              ctx.headers.append("X-Labels", "one");
              ctx.headers.append("x-labels", "two");
            }
          },
        },
      ],
    },
    decoy: { provider: "decoy", upstream: "wrong-model" },
  },
};
