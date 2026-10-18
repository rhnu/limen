import { defineGateway } from "limen:gateway";
import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "limen:policies";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", {
    endpoint,
    headers: { "X-Upstream-Profile": "static" },
  });
  provider0.use(capCompletionTokens({ name: "provider-cap", max: 64, onChange: "capped" }));
  provider0.use(
    dropRequestFields({ name: "privacy", fields: ["store", "metadata"], onChange: "scrubbed" }),
  );
  provider0.use(
    forwardClientHeader({ name: "profile", from: "X-Profile", to: "X-Upstream-Profile" }),
  );
  const model0 = provider0.model("chat", { upstream: "real-model" });
  model0.use(capCompletionTokens({ name: "model-cap", max: 16, onChange: "capped" }));
});
