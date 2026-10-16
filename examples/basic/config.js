import { capCompletionTokens } from "../../policies/index.js";

/** @satisfies {import("../../types/limen").Config} */
export default {
  providers: {
    primary: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: limen.env("UPSTREAM_API_KEY") ?? "example-upstream-token" },
      headers: { "X-App-Name": "Limen" },
      defaults: { temperature: 0.7 },
    },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "fictional-chat-model",
      defaults: { temperature: 0.4 },
      pipeline: [capCompletionTokens({ name: "limit-output", max: 4096 })],
    },
  },
};
