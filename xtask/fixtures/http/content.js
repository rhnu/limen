import { endpoint } from "./fixture-data.js";
import { replaceText } from "limen:content";

export default {
  providers: { p: { endpoint } },
  models: {
    chat: {
      provider: "p", upstream: "real-model",
      pipeline: [replaceText({
        name: "native-literal", search: "world", replacement: "reader", maxBufferedChars: 4,
      })],
    },
    script: {
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
