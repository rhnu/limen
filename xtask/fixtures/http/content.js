import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";
import { replaceText } from "limen:content";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", { endpoint });
  const model0 = provider0.model("chat", { upstream: "real-model" });
  model0.use(
    replaceText({
      name: "native-literal",
      search: "world",
      replacement: "reader",
      maxBufferedChars: 4,
    }),
  );
  const model1 = provider0.model("script", { upstream: "real-model" });
  model1.use({
    name: "semantic-literal",
    onContent(ctx) {
      if (ctx.event.type === "part_delta") {
        ctx.event.text = ctx.event.text.replaceAll("world", "reader");
      }
    },
  });
});
