import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  const provider0 = gateway.provider("p", { endpoint: "https://synthetic.invalid/chat" });
  provider0.use({
    name: "same",
    builtin: { kind: "replaceText", search: "ab", replacement: "bc", maxBufferedChars: 1 },
  });
  const model0 = provider0.model("chat", { upstream: "first" });
  model0.use({
    name: "same",
    builtin: { kind: "replaceText", search: "bc", replacement: "ZZ", maxBufferedChars: 1 },
  });
  const model1 = provider0.model("alias", { upstream: "second" });
  model1.use({
    name: "same",
    builtin: { kind: "replaceText", search: "bc", replacement: "ZZ", maxBufferedChars: 1 },
  });
});
