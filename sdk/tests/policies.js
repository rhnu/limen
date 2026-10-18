import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "limen:policies";

/** @typedef {{ calls?: number }} State */
/** @type {import("../limen").PipelineStep<State>[]} */
export const policies = [
  capCompletionTokens({ name: "budget", max: 64, onChange: "budget_capped" }),
  capCompletionTokens({ name: "legacy-budget", max: 64, field: "max_tokens" }),
  dropRequestFields({ name: "privacy", fields: ["store", "metadata"], onChange: "fields_removed" }),
  forwardClientHeader({ name: "client", from: "X-Profile", to: "x-profile" }),
];

// @ts-expect-error max must be numeric.
capCompletionTokens({ name: "bad", max: "64" });
// @ts-expect-error only the two explicit Chat token fields are supported.
capCompletionTokens({ name: "bad", max: 64, field: "output_tokens" });
// @ts-expect-error observation codes must be strings.
capCompletionTokens({ name: "bad", max: 64, onChange: 1 });
// @ts-expect-error field names are a list of strings.
dropRequestFields({ name: "bad", fields: [1] });
// @ts-expect-error forwarding requires an explicit destination.
forwardClientHeader({ name: "bad", from: "x-profile" });
// @ts-expect-error policies do not add arbitrary hooks or a composition schema.
capCompletionTokens({ name: "bad", max: 64, onRequest() {} });

// @ts-expect-error Native descriptors have no JavaScript hook to call.
policies[0].beforeSend({});
/** @type {import("../limen").PipelineStep} */
// @ts-expect-error A step must choose native execution or JavaScript hooks.
export const mixed = {
  name: "mixed",
  builtin: { kind: "capCompletionTokens", max: 1 },
  beforeSend() {},
};
/** @type {import("../limen").PipelineStep} */
// @ts-expect-error Unknown native descriptor kinds are rejected.
export const unknown = { name: "unknown", builtin: { kind: "unknown" } };
/** @type {import("../limen").PipelineStep} */
export const descriptor = {
  name: "literal",
  builtin: { kind: "dropRequestFields", fields: ["store"] },
};
