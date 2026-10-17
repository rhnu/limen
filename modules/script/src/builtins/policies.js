
/** @param {import("limen:policies").CapCompletionTokensOptions} options */
export function capCompletionTokens(options) {
  return descriptor(options, "capCompletionTokens", ["max", "field", "onChange"]);
}
/** @param {import("limen:policies").DropRequestFieldsOptions} options */
export function dropRequestFields(options) {
  return descriptor(options, "dropRequestFields", ["fields", "onChange"]);
}
/** @param {import("limen:policies").ForwardClientHeaderOptions} options */
export function forwardClientHeader(options) {
  return descriptor(options, "forwardClientHeader", ["from", "to"]);
}
