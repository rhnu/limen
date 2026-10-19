import { defineGateway } from "limen:gateway";
import { syntheticTextTool } from "limen:content";

export default defineGateway((gateway) => {
  const provider = gateway.provider("p", {
    endpoint: "https://upstream.invalid/chat",
  });
  provider
    .model("chat", { upstream: "native" })
    .use(syntheticTextTool({ name: "ordered", maxBufferedChars: 4096 }));
});
