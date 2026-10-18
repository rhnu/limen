import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "invalid-status",
    afterReceive(ctx) {
      ctx.status = 200;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
