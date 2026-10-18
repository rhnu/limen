import * as policies from "limen:policies";
import * as content from "limen:content";
const factories = [
  [policies.capCompletionTokens, { name: "cap", max: 1 }],
  [policies.dropRequestFields, { name: "drop", fields: ["store"] }],
  [policies.forwardClientHeader, { name: "header", from: "x-profile", to: "x-upstream" }],
  [content.replaceText, { name: "replace", search: "a", replacement: "b", maxBufferedChars: 1 }],
  [content.syntheticTextTool, { name: "tool", maxBufferedChars: 512 }],
];
let rejected = 0;
for (const [factory, options] of factories) {
  for (const value of [
    null,
    [],
    1,
    "private-value",
    Object.create({ name: "inherited" }),
    { ...options, stateKey: "old-state" },
    { ...options, unknown: true },
    { ...options, [Symbol()]: true },
  ]) {
    try {
      factory(value);
    } catch (error) {
      if (error instanceof TypeError) rejected++;
    }
  }
}
export default {
  rejected,
  hidden: policies.descriptor === undefined && content.descriptor === undefined,
};
