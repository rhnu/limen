import { defineGateway } from "limen:gateway";
import { capCompletionTokens } from "limen:policies";

/** @type {import("limen:gateway").GatewaySetup<{count?: number}>} */
const setup = (gateway) => {
  const provider = gateway.provider("primary", {
    endpoint: "https://fictional.invalid/chat",
    defaults: { temperature: 0.5 },
  });
  provider.use(capCompletionTokens({ name: "cap", max: 64 }));
  /** @type {undefined} */
  const used = provider.use(capCompletionTokens({ name: "another-cap", max: 32 }));
  void used;
  provider.step("concise", (step) =>
    step.beforeSend((ctx) => {
      ctx.state.count = 2;
    }),
  );
  const model = provider.model("chat", { upstream: "synthetic" });
  model.step("adapt", (step) => {
    step.beforeSend((ctx) => {
      ctx.state.count = 1;
      ctx.body.model = "upstream";
    });
    step.afterReceive((ctx) => {
      ctx.status = 201;
    });
    step.onResponse(async (ctx) => {
      await Promise.resolve();
      ctx.body.extension = true;
    });
    step.onContent((ctx) => {
      ctx.replaceEvents([ctx.event]);
    });
    step.onUpstreamEvent((ctx) => {
      ctx.event.model = "chat";
    });
    step.onStreamEvent((ctx) => {
      ctx.event.model = "chat";
    });
    step.onErrorResponse((ctx) => {
      ctx.status = 503;
    });
    // @ts-expect-error Lifecycle hooks belong to the gateway scope.
    step.onRequest(() => {});
    step.beforeSend((ctx) => {
      // @ts-expect-error beforeSend receives a request context, not response status.
      ctx.status = 200;
      // @ts-expect-error State retains the setup's JSON type.
      ctx.state.count = "one";
    });
    // @ts-expect-error Hook callbacks cannot return replacement objects.
    step.onResponse(() => ({}));
  });
  gateway.onRequest((ctx) => {
    ctx.state.count = 0;
  });
  gateway.onErrorResponse((ctx) => {
    ctx.status = 503;
  });
  gateway.onError((ctx) => {
    void ctx.error.code;
  });
  gateway.onFinish((ctx) => {
    void ctx.stats.elapsedMs;
  });
  // @ts-expect-error Providers are declared before models.
  gateway.model("chat", { upstream: "synthetic" });
  // @ts-expect-error Model scopes cannot register more models.
  model.model("another", { upstream: "synthetic" });
  // @ts-expect-error Options do not expose the removed pipeline object schema.
  gateway.provider("legacy", { endpoint: "https://fictional.invalid/", pipeline: [] });
  // @ts-expect-error The provider scope fixes model ownership.
  provider.model("legacy", { provider: "primary", upstream: "synthetic" });
  // @ts-expect-error Step setup must finish synchronously.
  model.step("async", async () => {});
  // @ts-expect-error Step setup cannot return values.
  model.step("value", () => ({}));
};
export const gateway = defineGateway(setup);

// @ts-expect-error Gateway setup must finish synchronously.
defineGateway(async () => {});
// @ts-expect-error Gateway setup cannot return configuration objects.
defineGateway(() => ({}));
// @ts-expect-error The public entry helper accepts a setup function.
defineGateway({ providers: {}, models: {} });
defineGateway((gateway) =>
  // @ts-expect-error Provider declarations return scopes, not undefined.
  gateway.provider("returned", { endpoint: "https://fictional.invalid/" }),
);
defineGateway((gateway) =>
  // @ts-expect-error Model declarations return scopes, not undefined.
  gateway
    .provider("returned", { endpoint: "https://fictional.invalid/" })
    .model("model", { upstream: "synthetic" }),
);
