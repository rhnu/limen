import { env } from "limen:runtime";
import { defineGateway } from "limen:gateway";
import { capCompletionTokens } from "limen:policies";

export default defineGateway((gateway) => {
  const primary = gateway.provider("primary", {
    endpoint: "https://fictional.invalid/v1/chat/completions",
    auth: { bearer: env("UPSTREAM_API_KEY") ?? "example-upstream-token" },
    headers: { "X-App-Name": "Limen" },
    defaults: { temperature: 0.7 },
  });
  const chat = primary.model("chat", {
    upstream: "fictional-chat-model",
    defaults: { temperature: 0.4 },
  });
  chat.use(capCompletionTokens({ name: "limit-output", max: 4096 }));
});
