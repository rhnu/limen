import * as policies from "limen:policies";
import * as content from "limen:content";
const repeated = await import("limen:policies");
export default {
  same: policies === repeated,
  policy: policies.capCompletionTokens({ name: "cap", max: 1 }).builtin.kind,
  content: content.replaceText({
    name: "replace",
    search: "a",
    replacement: "b",
    maxBufferedChars: 1,
  }).builtin.kind,
};
