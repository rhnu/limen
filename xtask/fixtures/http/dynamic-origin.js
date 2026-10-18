import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", {
    endpoint: "https://unused.invalid/chat/completions",
    auth: { bearer: "synthetic-upstream-secret" },
  });
  provider0.use({
    name: "dynamic-target",
    beforeSend(ctx) {
      ctx.url = endpoint;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
