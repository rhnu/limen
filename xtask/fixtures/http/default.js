import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", {
    endpoint,
    auth: { bearer: "synthetic-upstream-secret" },
    defaults: { temperature: 0.2, extra: { provider: true } },
  });
  const model0 = provider0.model("chat", {
    upstream: "real-model",
    defaults: { temperature: 0.4 },
  });
});
