import { defineGateway } from "limen:gateway";
import { mapPartText, replaceText, syntheticTextTool } from "limen:content";

// Use any alias with either client stream setting. Upstream JSON/SSE is independent.
const replacement = replaceText({
  name: "replace-public-label",
  search: "world",
  replacement: "reader",
  maxBufferedChars: 4,
});

export default defineGateway((gateway) => {
  const demo = gateway.provider("demo", {
    endpoint: "https://example.invalid/v1/chat/completions",
  });
  const chat = demo.model("chat", { upstream: "demo-model" });
  chat.use(syntheticTextTool({ name: "text-transport", maxBufferedChars: 1024 }));
  chat.use(replacement);

  const literal = demo.model("literal", { upstream: "demo-model" });
  literal.use(replacement);

  const complete = demo.model("complete", { upstream: "demo-model" });
  complete.use(
    mapPartText({
      name: "complete-part-regex",
      kinds: ["text"],
      maxBufferedChars: 4096,
      transform: (text) => text.replace(/\bworld\b/g, "reader"),
    }),
  );
});
