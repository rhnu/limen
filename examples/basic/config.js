import { limitOutput } from "../shared/rules.mjs";

export default {
  providers: {
    primary: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: "example-upstream-token" },
      headers: { "X-App-Name": "Limen" },
      defaults: { temperature: 0.7 },
    },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "fictional-chat-model",
      defaults: { temperature: 0.4 },
      pipeline: [limitOutput],
    },
  },
};
