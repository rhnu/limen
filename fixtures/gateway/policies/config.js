import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "limen:policies";
import { endpoint } from "../endpoint.js";

export default {
  providers: {
    p: {
      endpoint,
      headers: { "X-Upstream-Profile": "static" },
      pipeline: [
        capCompletionTokens({ name: "provider-cap", max: 64, onChange: "capped" }),
        dropRequestFields({ name: "privacy", fields: ["store", "metadata"], onChange: "scrubbed" }),
        forwardClientHeader({ name: "profile", from: "X-Profile", to: "X-Upstream-Profile" }),
      ],
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "real-model",
      pipeline: [capCompletionTokens({ name: "model-cap", max: 16, onChange: "capped" })],
    },
  },
};
