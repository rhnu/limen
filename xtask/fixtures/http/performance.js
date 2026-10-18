import { defineGateway } from "limen:gateway";
import { capCompletionTokens } from "limen:policies";
import { endpoint } from "./fixture-data.js";

export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    ctx.body.temperature = 0.7;
  });
  gateway.onFinish(() => {});
  const provider = gateway.provider("p", { endpoint, defaults: { top_p: 0.8 } });
  provider.use(capCompletionTokens({ name: "budget", max: 16, field: "max_tokens" }));
  const model = provider.model("chat", { upstream: "real-model" });
  model.use({
    name: "controls",
    beforeSend(ctx) {
      ctx.body.temperature = 0.9;
    },
  });
});
