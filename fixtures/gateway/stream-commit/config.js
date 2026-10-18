function copy(event) {
  return JSON.parse(JSON.stringify(event));
}

// The JSON payload fits exactly; its SSE prefix and delimiters do not.
function fillPayload(event) {
  event.choices[0].delta.content = "";
  event.choices[0].delta.content = "x".repeat(512 - JSON.stringify(event).length);
}

export default {
  onRequest(ctx) {
    ctx.state.mode = ctx.body.mode;
  },
  onFinish(ctx) {
    ctx.observe.record("finish");
  },
  providers: {
    p: {
      endpoint: "https://example.invalid/chat",
      pipeline: [
        {
          name: "provider",
          onStreamEvent(ctx) {
            ctx.state.providerCalls = (ctx.state.providerCalls ?? 0) + 1;
            ctx.observe.record("provider");
            if (ctx.state.mode === "provider-framing") fillPayload(ctx.event);
          },
        },
      ],
    },
  },
  models: {
    provider: { provider: "p", upstream: "upstream" },
    chat: {
      provider: "p",
      upstream: "upstream",
      pipeline: [
        {
          name: "model",
          onStreamEvent(ctx) {
            ctx.state.modelCalls = (ctx.state.modelCalls ?? 0) + 1;
            ctx.observe.record("model");
            ctx.observe.record("private");
            if (ctx.state.mode === "framing") fillPayload(ctx.event);
            if (ctx.state.mode === "finished-choice") {
              ctx.event.choices[0].finish_reason = "stop";
            }
            if (ctx.state.mode === "batch-finished") {
              ctx.replaceEvents([copy(ctx.event), copy(ctx.event)]);
            }
            if (ctx.state.mode === "batch-framing") {
              const second = copy(ctx.event);
              second.choices[0].index = 1;
              fillPayload(second);
              ctx.replaceEvents([copy(ctx.event), second]);
            }
            if (ctx.state.mode === "split") {
              ctx.replaceEvents(
                ctx.event.choices.map((choice) => {
                  const event = copy(ctx.event);
                  event.choices = [choice];
                  return event;
                }),
              );
            }
          },
        },
      ],
    },
  },
};
