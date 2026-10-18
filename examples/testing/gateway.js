import { defineGateway } from "limen:gateway";

// `limen test --config examples/testing/server.toml --cases examples/testing`
// uses the fixture responses and never connects to this reserved example host.
export default defineGateway((gateway) => {
  const demo = gateway.provider("demo", {
    endpoint: "https://example.invalid/v1/chat/completions",
  });
  demo.step("request-policy", (step) => {
    step.beforeSend(async (ctx) => {
      await Promise.resolve();
      delete ctx.body.store;
      ctx.headers.set("X-Policy", "checked");
    });
  });
  demo.model("chat", { upstream: "demo-model" });
});
