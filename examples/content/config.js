import { mapPartText, replaceText, syntheticTextTool } from "limen:content";

// Use either alias with either client stream setting. Upstream JSON/SSE is independent.
const replacement = replaceText({
  name: "replace-public-label",
  search: "world",
  replacement: "reader",
  maxBufferedChars: 4,
});

/** @satisfies {import("../../sdk/limen").Config} */
export default {
  providers: { demo: { endpoint: "https://example.invalid/v1/chat/completions" } },
  models: {
    chat: {
      provider: "demo",
      upstream: "demo-model",
      pipeline: [
        syntheticTextTool({ name: "text-transport", maxBufferedChars: 1024 }),
        replacement,
      ],
    },
    literal: { provider: "demo", upstream: "demo-model", pipeline: [replacement] },
    complete: {
      provider: "demo",
      upstream: "demo-model",
      pipeline: [
        mapPartText({
          name: "complete-part-regex",
          kinds: ["text"],
          maxBufferedChars: 4096,
          transform: (text) => text.replace(/\bworld\b/g, "reader"),
        }),
      ],
    },
  },
};
