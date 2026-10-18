import * as first from "limen:policies";
import * as gateway from "limen:gateway";
import { capCompletionTokens as repeated } from "./reexport.js";
import { capCompletionTokens as local } from "./policies.js";
const dynamic = await import("limen:policies");
const dynamicGateway = await import("limen:gateway");
export default () => ({
  same: first === dynamic && first.capCompletionTokens === repeated && gateway === dynamicGateway,
  local,
  kind: repeated({ name: "cap", max: 1 }).builtin.kind,
});
