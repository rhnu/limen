import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", { endpoint: "https://synthetic.invalid/chat" });
  provider0.use({
    name: "replace",
    builtin: { kind: "replaceText", search: "ab", replacement: "ZZ", maxBufferedChars: 1 },
  });
  const model0 = provider0.model("chat", { upstream: "native" });
});
