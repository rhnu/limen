import { env } from "limen:runtime";
import { defineGateway } from "limen:gateway";
/** @typedef {{ calls?: number, details?: { tags: string[] } }} RequestState */

export const config = defineGateway(
  /** @param {import("../limen").GatewayScope<RequestState>} gateway */ (gateway) => {
    const primary = gateway.provider("primary", {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: env("UPSTREAM_API_KEY") ?? "example-upstream-token" },
      headers: { "X-App-Name": "Limen" },
      defaults: { temperature: 0.7, extension: { enabled: true, values: [null, 1] } },
    });
    primary.use({
      name: "provider",
      beforeSend(ctx) {
        if (ctx.client.headers.getAll("x-client-name").includes("unsupported"))
          return ctx.reject(403);
        ctx.state.calls = (ctx.state.calls ?? 0) + 1;
        ctx.state.details = { tags: [ctx.meta.provider] };
        ctx.body.max_completion_tokens = 1024;
        delete ctx.body.store;
        ctx.url = "https://fictional.invalid/custom/chat/completions";
        ctx.headers.append("x-trace", ctx.meta.requestId);
        ctx.headers.set("authorization", "Bearer example-upstream-token");
        ctx.headers.delete("x-obsolete");
        ctx.headers.getAll("x-trace").push("detached-array");
        /** @type {null} */
        const status = ctx.meta.upstreamStatus;
        void status;
      },
      async afterReceive(ctx) {
        await Promise.resolve();
        if (ctx.meta.upstreamStatus === 429) ctx.status = 503;
        ctx.headers.set("retry-after", "3");
        ctx.state.calls = 2;
      },
      onResponse(ctx) {
        ctx.body.extension = { source: ctx.meta.upstreamContentType, values: [null, true] };
        ctx.body = { ...ctx.body, model: ctx.meta.model };
      },
      async onUpstreamEvent(ctx) {
        await Promise.resolve();
        ctx.event.extension = { phase: ctx.meta.phase };
        ctx.replaceEvents([ctx.event, { ...ctx.event, choices: [] }]);
      },
      onStreamEvent(ctx) {
        if (ctx.event.choices?.length === 0) ctx.replaceEvents([]);
        else ctx.event.model = ctx.meta.model;
      },
    });
    const chat = primary.model("chat", {
      upstream: "fictional-chat-model",
      defaults: { max_completion_tokens: 2048 },
    });
    chat.use({ name: "model", beforeSend: plainNoReturn, onResponse: asyncNoReturn });

    gateway.onRequest((ctx) => {
      if (ctx.body.unsupported === true) return ctx.reject();
      ctx.client.headers.getAll("X-Client-Name").push("detached-array");
      ctx.body.model = "chat";
      ctx.body.messages.push({ role: "system", content: "Example" });
      ctx.body.stream = false;
      ctx.state.calls = 0;
      /** @type {null} */
      const provider = ctx.meta.provider;
      /** @type {boolean} */
      const originalIntent = ctx.meta.clientStream;
      void provider;
      void originalIntent;
    });

    gateway.onError((ctx) => {
      /** @type {"failed" | "cancelled"} */
      const outcome = ctx.outcome;
      /** @type {string} */
      const code = ctx.error.code;
      /** @type {string | null} */
      const model = ctx.meta.model;
      void [
        outcome,
        code,
        model,
        ctx.error.status,
        ctx.error.request_id,
        ctx.error.phase,
        ctx.error.step,
      ];
    });

    gateway.onErrorResponse(async (ctx) => {
      await Promise.resolve();
      ctx.status = 503;
      ctx.headers.set("retry-after", "5");
      ctx.body = ctx.body === null ? null : JSON.stringify({ message: ctx.body });
      ctx.contentType = ctx.body === null ? null : "application/json";
      ctx.state.calls = (ctx.state.calls ?? 0) + 1;
      ctx.state.details?.tags.push("error-translated");
      ctx.observe.record("error_translated");
      /** @type {string | null} */
      const provider = ctx.meta.provider;
      /** @type {"onErrorResponse"} */
      const phase = ctx.meta.phase;
      void [provider, phase, ctx.error.code, ctx.client.headers.getAll("x-client-name")];
    });

    gateway.onFinish(async (ctx) => {
      ctx.observe.record("request_finished");
      await Promise.resolve();
      if (ctx.outcome !== "success") {
        /** @type {string} */
        const message = ctx.error.message;
        void message;
      }
      const tags = ctx.state.details?.tags;
      const usage = ctx.stats.usage;
      const timing = ctx.stats.timings.hooks.beforeSend;
      void [
        usage.quality,
        usage.cachedInputTokens,
        ctx.stats.timings.requestMs,
        timing?.queue.samples,
      ];
      tags?.map((tag) => tag.toUpperCase());
      void [
        ctx.stats.inputBytes,
        ctx.stats.outboundBytes,
        ctx.stats.upstreamBytes,
        ctx.stats.outputBytes,
        ctx.stats.upstreamHeadersMs,
        ctx.stats.elapsedMs,
        ctx.stats.finalUpstreamModel,
        ctx.stats.target,
      ];
    });
  },
);

/** @param {import("../limen").BeforeSendContext<RequestState>} ctx */
function plainNoReturn(ctx) {
  ctx.body.extension = [null, { enabled: true }];
}

/** @param {import("../limen").ResponseContext<RequestState>} ctx */
async function asyncNoReturn(ctx) {
  await Promise.resolve();
  ctx.state.calls = 3;
}

/** @type {import("../limen").StreamEvent} */
export const event = {
  id: "example",
  object: "chat.completion.chunk",
  created: 1,
  model: "chat",
  choices: [
    {
      index: 0,
      delta: {
        role: "assistant",
        content: null,
        refusal: null,
        tool_calls: [
          {
            index: 0,
            id: "call",
            type: "function",
            function: { name: "example", arguments: "{}" },
          },
        ],
        extension: [true],
      },
      finish_reason: "tool_calls",
      extension: { ok: true },
    },
  ],
  usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
  extension: [null, "preserved"],
};

/** @type {import("../limen").PipelineStep} */
export const untypedState = {
  name: "default-state",
  beforeSend(ctx) {
    ctx.state.extension = { nested: [1, "value"] };
    const count = ctx.state.count;
    if (typeof count === "number") ctx.state.count = count + 1;
  },
};

/** @type {import("../limen").PipelineStep} */
export const asyncRejection = {
  name: "async-rejection",
  async beforeSend(ctx) {
    await Promise.resolve();
    if (ctx.body.unsupported === true) return ctx.reject(422);
  },
};

/** @type {import("../limen").PipelineStep<RequestState>} */
export const errorResponseOnly = {
  name: "error-response",
  onErrorResponse(ctx) {
    ctx.body = "Translated upstream failure";
    ctx.contentType = "text/plain";
    ctx.status = 502;
    ctx.state.details = { tags: [ctx.error.code] };
  },
};
