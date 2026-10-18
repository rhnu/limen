import * as first from "limen:policies";
import { capCompletionTokens as repeated } from "./reexport.js";
import { capCompletionTokens as local } from "./policies.js";
const dynamic = await import("limen:policies");
export default {
  same: first === dynamic && first.capCompletionTokens === repeated,
  local,
  kind: repeated({ name: "cap", max: 1 }).builtin.kind,
};
