import { env } from "limen:runtime";
import { defineGateway } from "limen:gateway";
// Each expected diagnostic protects an author-facing contract; no runtime execution.
/** @typedef {{ calls?: number, details?: { tags: string[] } }} RequestState */

/** @type {import("../limen").PipelineStep} */
export const syncReturn = {
  name: "sync-return",
  // @ts-expect-error Hooks cannot return a replacement draft.
  beforeSend() {
    return { model: "replacement" };
  },
};

/** @type {import("../limen").PipelineStep} */
export const asyncReturn = {
  name: "async-return",
  // @ts-expect-error Promise completion values must be undefined.
  async beforeSend() {
    return { model: "replacement" };
  },
};

/** @type {import("../limen").PipelineStep} */
export const nullReturn = {
  name: "null-return",
  // @ts-expect-error null is not the no-return completion value.
  afterReceive() {
    return null;
  },
};

/** @type {import("../limen").PipelineStep} */
export const arrayReturn = {
  name: "array-return",
  // @ts-expect-error Streaming output uses replaceEvents, not returned arrays.
  async onStreamEvent() {
    return [];
  },
};
export const config = defineGateway(
  /** @param {import("../limen").GatewayScope<RequestState>} gateway */ (gateway) => {
    const primary = gateway.provider("primary", {
      endpoint: "https://fictional.invalid/v1/chat/completions",
    });
    const chat = primary.model("chat", { upstream: "fictional-chat-model" });

    gateway.onRequest((ctx) => {
      // @ts-expect-error Observation codes must be strings.
      ctx.observe.record(1);
      // @ts-expect-error Observation records have no arbitrary payload.
      ctx.observe.record("policy_applied", { body: ctx.body });
      // @ts-expect-error The observation method is frozen.
      ctx.observe.record = () => {};
      // @ts-expect-error The observation API reference is fixed.
      ctx.observe = ctx.observe;
      // @ts-expect-error Only supported business rejection statuses are accepted.
      ctx.reject(500);
      // @ts-expect-error Rejection does not accept arbitrary response bodies or messages.
      ctx.reject({ status: 400, message: "private" });
      // @ts-expect-error The rejection method cannot be replaced.
      ctx.reject = () => {};
      // @ts-expect-error The client context reference is fixed.
      ctx.client = ctx.client;
      // @ts-expect-error The read-only client header view cannot be replaced.
      ctx.client.headers = ctx.client.headers;
      // @ts-expect-error Client header views expose no mutations.
      ctx.client.headers.set("x-test", "changed");
      // @ts-expect-error Client header methods are frozen.
      ctx.client.headers.getAll = () => [];
      // @ts-expect-error Client header names must be strings.
      ctx.client.headers.getAll(1);
      // @ts-expect-error Header controls exist only in beforeSend and afterReceive.
      ctx.headers.set("x-test", "value");
      // @ts-expect-error Routing is not available yet.
      ctx.meta.provider.toUpperCase();
      // @ts-expect-error Metadata is frozen.
      ctx.meta.clientStream = true;
      // @ts-expect-error Metadata reference is fixed.
      ctx.meta = ctx.meta;
      // @ts-expect-error State reference is fixed even in mutable phases.
      ctx.state = {};
      // @ts-expect-error Declared state fields retain their types.
      ctx.state.calls = "many";
      // @ts-expect-error The JSON object body cannot become an array.
      ctx.body = [];
      // @ts-expect-error Original stream intent and the upstream stream draft are booleans.
      ctx.body.stream = "true";
      // @ts-expect-error JSON extensions cannot be functions.
      ctx.body.extension = () => true;
      // @ts-expect-error JSON extensions cannot contain undefined.
      ctx.body.extension = { nested: undefined };
      // @ts-expect-error Methods for event output are phase-specific.
      ctx.replaceEvents([]);
    });

    gateway.onError((ctx) => {
      // @ts-expect-error Observers cannot reject a finished request.
      ctx.reject(400);
      // @ts-expect-error Observer state is deeply readonly.
      ctx.state.calls = 1;
      // @ts-expect-error Nested arrays in observer state are frozen too.
      ctx.state.details?.tags.push("changed");
      // @ts-expect-error Error metadata cannot be changed.
      ctx.error.code = "cancelled";
      // @ts-expect-error Statistics cannot be changed.
      ctx.stats.elapsedMs = 0;
      // @ts-expect-error nested statistics are read-only
      ctx.stats.usage.inputTokens = 1;
      // @ts-expect-error nested timing samples are read-only
      ctx.stats.timings.hooks.beforeSend.queue.samples = 1;
      // @ts-expect-error Observers do not receive draft bodies.
      ctx.body = {};
      /** @type {"success"} */
      // @ts-expect-error onError never observes success.
      const outcome = ctx.outcome;
      void outcome;
    });

    gateway.onErrorResponse((ctx) => {
      // @ts-expect-error The original error classification is readonly.
      ctx.error.code = "internal_error";
      // @ts-expect-error The error reference is fixed.
      ctx.error = ctx.error;
      // @ts-expect-error Metadata remains readonly while editing an error response.
      ctx.meta.phase = "onErrorResponse";
      // @ts-expect-error The metadata reference is fixed.
      ctx.meta = ctx.meta;
      // @ts-expect-error The state reference is fixed even though its fields are mutable.
      ctx.state = {};
      // @ts-expect-error Error bodies are nullable text, not JSON objects.
      ctx.body = { error: "changed" };
      // @ts-expect-error A binary error body may not be text.
      ctx.body.toUpperCase();
      // @ts-expect-error Content-Type must be text or null.
      ctx.contentType = 1;
      // @ts-expect-error Status remains numeric.
      ctx.status = "503";
      // @ts-expect-error Header draft references cannot be replaced.
      ctx.headers = ctx.headers;
      // @ts-expect-error Header methods are frozen.
      ctx.headers.set = () => {};
      // @ts-expect-error Routing may not have happened.
      ctx.meta.provider.toUpperCase();
      // @ts-expect-error This phase cannot record request rejection.
      ctx.reject(400);
      /** @type {import("../limen").RequestHookPhase} */
      // @ts-expect-error Request timings exclude cleanup phases.
      const timing = "onErrorResponse";
      void timing;
    });

    gateway.onFinish((ctx) => {
      // @ts-expect-error A successful outcome has no error object.
      ctx.error.code.toUpperCase();
      // @ts-expect-error The terminal outcome cannot be changed.
      ctx.outcome = "failed";
      // @ts-expect-error An observer has no header controls.
      ctx.headers.set("x-test", "value");
    });
  },
);

/** @type {import("../limen").PipelineStep<RequestState>} */
export const phases = {
  name: "phases",
  beforeSend(ctx) {
    // @ts-expect-error Header draft references cannot be replaced.
    ctx.headers = ctx.headers;
    // @ts-expect-error Header API methods are frozen.
    ctx.headers.set = () => {};
    // @ts-expect-error Header values are strings.
    ctx.headers.set("x-count", 1);
    // @ts-expect-error Header APIs have no direct field assignment interface.
    ctx.headers.authorization = "Bearer example";
    // @ts-expect-error Status only exists in afterReceive.
    ctx.status = 200;
    // @ts-expect-error Header receipt has not occurred yet.
    ctx.meta.upstreamStatus.toFixed();
    // @ts-expect-error The outgoing URL must be a string.
    ctx.url = 12;
  },
  afterReceive(ctx) {
    // @ts-expect-error Rejection is only available before upstream dispatch.
    ctx.reject(403);
    // @ts-expect-error afterReceive does not have a body draft.
    ctx.body.model = "chat";
    // @ts-expect-error afterReceive does not have a URL draft.
    ctx.url = "https://fictional.invalid/";
    // @ts-expect-error Status is numeric.
    ctx.status = "200";
    // @ts-expect-error Original upstream facts remain frozen.
    ctx.meta.upstreamStatus = 200;
  },
  onResponse(ctx) {
    // @ts-expect-error Rejection is unavailable in response hooks.
    ctx.reject();
    // @ts-expect-error Only stream hooks receive event output controls.
    ctx.replaceEvents([]);
    // @ts-expect-error Response hooks cannot alter status.
    ctx.status = 201;
  },
  onUpstreamEvent(ctx) {
    // @ts-expect-error Rejection is unavailable during streaming.
    ctx.reject(422);
    // @ts-expect-error Event drafts must be JSON chunk objects.
    ctx.event = null;
    // @ts-expect-error Control frames belong to Rust, not JSON event output.
    ctx.replaceEvents(["[DONE]"]);
    // @ts-expect-error Choice indices remain numeric in open event objects.
    ctx.replaceEvents([{ choices: [{ index: "invalid" }] }]);
    // @ts-expect-error The original SSE envelope is read-only.
    ctx.sse = null;
    // @ts-expect-error replaceEvents accepts an array, not a single event.
    ctx.replaceEvents(ctx.event);
    // @ts-expect-error replaceEvents itself cannot be replaced.
    ctx.replaceEvents = () => {};
    // @ts-expect-error Event phases have no request/response body draft.
    ctx.body = {};
  },
  onStreamEvent(ctx) {
    // @ts-expect-error Standard event fields have known types.
    ctx.event.model = false;
    // @ts-expect-error Event extensions must be JSON.
    ctx.event.extension = new Date();
    // @ts-expect-error Header mutations are not available while streaming.
    ctx.headers.delete("x-test");
  },
};
export const unknownField = defineGateway(
  /** @param {import("../limen").GatewayScope<import("../limen").JsonObject>} gateway */ (
    gateway,
  ) => {
    // @ts-expect-error Unknown configuration hooks are rejected.

    gateway.onRequests(() => {});
  },
);

/** @type {import("../limen").ProviderOptions} */
export const provider = {
  endpoint: "https://fictional.invalid/",
  // @ts-expect-error Static headers require string values.
  headers: { "x-test": 1 },
};

/** @type {import("../limen").Defaults} */
export const defaults = {
  // @ts-expect-error Defaults cannot override routing control fields.
  model: "chat",
};

/** @type {import("../limen").ModelOptions} */
export const model = {
  upstream: "model",
  // @ts-expect-error Models have no independent endpoint field.
  endpoint: "https://fictional.invalid/",
};

/** @type {import("../limen").PipelineStep} */
// @ts-expect-error A pipeline step must define at least one phase hook.
export const emptyStep = { name: "empty" };

/** @type {import("../limen").PipelineStep} */
export const errorResponseReturn = {
  name: "error-return",
  // @ts-expect-error Error response hooks cannot return replacement bodies.
  onErrorResponse() {
    return "replacement";
  },
};
export const errorResponseAsyncReturn = defineGateway(
  /** @param {import("../limen").GatewayScope<import("../limen").JsonObject>} gateway */ (
    gateway,
  ) => {
    // @ts-expect-error Async error response completion must be undefined.

    gateway.onErrorResponse(async () => {
      return {};
    });
  },
);
// @ts-expect-error State must contain JSON data.
/** @type {import("../limen").GatewayScope<{ callback: () => void }>} */
export const nonJsonState = /** @type {never} */ (null);

// @ts-expect-error Environment names must be strings.
env(1);
// @ts-expect-error Environment results can be missing.
env("MISSING").toUpperCase();
// @ts-expect-error Imported bindings are read-only.
env = () => "changed";
// @ts-expect-error QuickJS has no Node.js process host API.
process.env.UPSTREAM_API_KEY;
// @ts-expect-error Network host APIs are not exposed.
fetch("https://fictional.invalid/");
// @ts-expect-error DOM globals are not exposed.
document.title;
// @ts-expect-error Browser/Node timers are not exposed.
setTimeout(() => {}, 1);

// @ts-expect-error Embedded factory helpers are private implementation details.
descriptor({ name: "hidden", max: 1 }, "capCompletionTokens", ["max"]);
