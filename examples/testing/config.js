// `limen test --config examples/testing/server.toml --cases examples/testing`
// uses the fixture responses and never connects to this reserved example host.
/** @satisfies {import("../../sdk/limen").Config} */
export default {
  providers: {
    demo: {
      endpoint: "https://example.invalid/v1/chat/completions",
      pipeline: [{
        name: "request-policy",
        async beforeSend(ctx) {
          await Promise.resolve();
          delete ctx.body.store;
          ctx.headers.set("X-Policy", "checked");
        },
      }],
    },
  },
  models: { chat: { provider: "demo", upstream: "demo-model" } },
};
