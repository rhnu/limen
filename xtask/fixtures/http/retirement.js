import { defineGateway } from "limen:gateway";
import { endpoint } from "./fixture-data.js";

export default defineGateway((gateway) => {
  gateway.onRequest(function () {
    while (true) {}
  });
  const provider0 = gateway.provider("p", { endpoint });
  const model0 = provider0.model("chat", { upstream: "real-model" });
});
