import { endpoint } from "../endpoint.js";

let requests = 0;

export default {
  onRequest(ctx) {
    requests++;
    ctx.state.requests = requests;
    ctx.state.suffix = ctx.body.suffix || "";
  },
  providers: {
    p: {
      endpoint,
      pipeline: [{
        name: "request-state",
        onResponse(ctx) {
          ctx.body.choices[0].message.content += ctx.state.suffix;
        },
      }],
    },
  },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
