import { endpoint } from "./fixture-data.js";

export default {
  providers: { p: { endpoint } },
  models: {
    chat: {
      provider: "p", upstream: "real-model",
      pipeline: [{
        name: "semantic-literal",
        onContent(ctx) {
          if (ctx.event.type === "part_delta") {
            ctx.event.text = ctx.event.text.replaceAll("world", "reader");
          }
        },
      }],
    },
  },
};
