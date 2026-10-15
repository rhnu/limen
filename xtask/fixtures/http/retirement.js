import { endpoint } from "./fixture-data.js";

export default {
  onRequest() {
    while (true) {}
  },
  providers: { p: { endpoint } },
  models: { chat: { provider: "p", upstream: "real-model" } },
};
