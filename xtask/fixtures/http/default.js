import { endpoint } from "./fixture-data.js";

export default {
  providers: {
    p: {
      endpoint,
      auth: { bearer: "synthetic-upstream-secret" },
      defaults: { temperature: 0.2, extra: { provider: true } },
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "real-model",
      defaults: { temperature: 0.4 },
    },
  },
};
