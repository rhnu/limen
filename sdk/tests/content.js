import { mapPartText, replaceText, syntheticTextTool } from "limen:content";

/** @type {import("../limen").PipelineStep} */
export const content = {
  name: "semantic",
  onContent(ctx) {
    ctx.state.seen = true;
    if (ctx.event.type === "part_start" && ctx.event.kind === "tool") {
      ctx.event.tool.name = "function_name";
      ctx.event.tool.index = 1;
    }
    if (ctx.event.type === "part_delta") ctx.event.text = ctx.event.text.toUpperCase();
    if (ctx.event.type === "choice_end") ctx.event.finishReason = "stop";
    ctx.replaceEvents([ctx.event]);
    // @ts-expect-error semantic events are independent of raw Chat payloads.
    ctx.event.choices = [];
    // @ts-expect-error payload events belong to the low-level hooks.
    ctx.replaceEvents([{ choices: [] }]);
    // @ts-expect-error finishReason belongs only to choice_end.
    ctx.event.finishReason = "stop";
    // @ts-expect-error original SSE envelope is readonly.
    ctx.sse = null;
    // @ts-expect-error state reference is fixed.
    ctx.state = {};
    // @ts-expect-error no raw body is exposed here.
    ctx.body = {};
  },
};

/** @type {import("../limen").PipelineStep[]} */
export const policies = [
  mapPartText({
    name: "whole",
    kinds: ["text", "tool"],
    maxBufferedChars: 4096,
    transform(value, info) {
      // @ts-expect-error callback metadata is immutable.
      info.part = "changed";
      return info.kind === "tool" ? value : value.trim();
    },
  }),
  replaceText({ name: "redact", search: "public", replacement: "visible", maxBufferedChars: 16 }),
  syntheticTextTool({ name: "unwrap", toolName: "emit_text", maxBufferedChars: 512 }),
];
mapPartText({
  name: "bad",
  kinds: ["text"],
  maxBufferedChars: 1,
  // @ts-expect-error complete-part callbacks must be synchronous.
  transform: async (value) => value,
});
// @ts-expect-error complete-part callbacks must return text.
mapPartText({ name: "bad", kinds: ["text"], maxBufferedChars: 1, transform: () => 1 });
// @ts-expect-error only semantic text-bearing part kinds can be selected.
mapPartText({ name: "bad", kinds: ["audio"], maxBufferedChars: 1, transform: (value) => value });
// @ts-expect-error retained buffering bound must be explicit.
replaceText({ name: "redact", search: "a", replacement: "b" });
// @ts-expect-error replacement is literal text, never a callback.
replaceText({ name: "redact", search: "a", replacement: () => "b", maxBufferedChars: 1 });
// @ts-expect-error no host I/O or retry callbacks are part of this transport.
syntheticTextTool({ name: "tool", maxBufferedChars: 512, retry() {} });
/** @type {import("../limen").ContentEvent} */
// @ts-expect-error a tool start requires typed metadata and a stable index.
export const missingTool = { type: "part_start", choice: 0, part: "tool", kind: "tool" };
/** @type {import("../limen").ContentEvent} */
// @ts-expect-error control markers remain gateway owned.
export const done = { type: "done" };

replaceText({
  name: "bad",
  search: "a",
  replacement: "b",
  maxBufferedChars: 1,
  // @ts-expect-error Native state is private to Rust.
  stateKey: "custom",
});
// @ts-expect-error Native tool state is private to Rust.
syntheticTextTool({ name: "bad", maxBufferedChars: 512, stateKey: "custom" });
// Callback policies retain explicit request-state configuration.
mapPartText({
  name: "custom",
  kinds: ["text"],
  stateKey: "custom",
  maxBufferedChars: 8,
  transform: (value) => value,
});
// @ts-expect-error Only explicitly registered embedded modules exist.
import { unknown } from "limen:unknown";
