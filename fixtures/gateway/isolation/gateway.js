import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

let requests = 0;

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    requests++;
    ctx.state.requests = requests;
    ctx.state.suffix = ctx.body.suffix || "";
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "request-state",
    onResponse(ctx) {
      ctx.body.choices[0].message.content += ctx.state.suffix;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
