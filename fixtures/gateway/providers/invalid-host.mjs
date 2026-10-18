import { defineGateway } from "limen:gateway";
import { createOpenAI, createDeepSeek, createOpenRouter } from "limen:providers";

// Factories preserve host validation of connection fields and open defaults.
const factories = [createOpenAI, createDeepSeek, createOpenRouter];
const invalid = [
  { endpoint: "not-a-url" },
  { apiKey: "secret-token\ninvalid" },
  { headers: { Host: "example.invalid" } },
  { headers: { "X-Custom": "one", "x-custom": "two" } },
  { defaults: { model: "forbidden" } },
  { defaults: { stream: true } },
  { defaults: { messages: [] } },
  { defaults: { extension: Number.NaN } },
  {
    defaults: {
      extension: {
        get secret() {
          throw new Error("secret-getter");
        },
      },
    },
  },
];

export function invalidGateway(factory, index) {
  return defineGateway((gateway) => {
    factories[factory](gateway, "p", { apiKey: "synthetic-key", ...invalid[index] }).model("chat", {
      upstream: "synthetic-model",
    });
  });
}
