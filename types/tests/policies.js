import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "../../policies/index.js";

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
