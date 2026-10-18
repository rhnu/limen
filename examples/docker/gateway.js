import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  const primary = gateway.provider("primary", {
    endpoint: "https://fictional.invalid/v1/chat/completions",
    auth: { bearer: limen.env("UPSTREAM_API_KEY") ?? "example-upstream-token" },
  });
  primary.model("chat", { upstream: "fictional-chat-model" });
});
