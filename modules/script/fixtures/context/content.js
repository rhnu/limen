let release;
let started = false;
let finished = false;
async function content(ctx) {
  ctx.state.visits = (ctx.state.visits ?? 0) + 1;
  switch (ctx.meta.operation) {
    case "edit":
      ctx.event.nested.value++;
      ctx.event.extensions = { custom: [true, null, "owned"] };
      ctx.state.source = ctx.sse;
      break;
    case "count":
      ctx.replaceEvents(Array(ctx.meta.count).fill({ extension: true }));
      break;
    case "late-count": {
      const events = [{}];
      ctx.replaceEvents(events);
      await Promise.resolve();
      events.push({});
      break;
    }
    case "throw":
      throw new Error("private-content-error");
    case "reject":
      await Promise.reject(new Error("private-content-error"));
      break;
    case "return":
      return {};
    case "invalid":
      ctx.event = { invalid: () => {} };
      break;
    case "body":
      ctx.body = {};
      break;
    case "meta":
      ctx.meta.clientStream = true;
      break;
    case "sse-ref":
      ctx.sse = null;
      break;
    case "sse-field":
      ctx.sse.id = "changed";
      break;
    case "sse-nested":
      ctx.sse.extra.value++;
      break;
    case "state-ref":
      ctx.state = {};
      break;
    case "pending":
      ctx.replaceEvents([{ uncommitted: true }]);
      await new Promise((resolve) => {
        started = true;
        release = resolve;
      });
      ctx.state.late = true;
      finished = true;
      break;
    default:
      ctx.state.source = ctx.sse;
      break;
  }
}
export default {
  providers: {
    p: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      pipeline: [
        {
          name: "provider",
          onContent: content,
          onResponse(ctx) {
            ctx.body.lower = "response";
          },
          onUpstreamEvent(ctx) {
            ctx.event.lower = "upstream";
          },
          onStreamEvent(ctx) {
            ctx.event.lower = "stream";
          },
        },
      ],
    },
  },
  models: {
    m: {
      provider: "p",
      upstream: "synthetic",
      pipeline: [
        {
          name: "model",
          onContent(ctx) {
            ctx.state.visits++;
            ctx.event.model = true;
          },
        },
      ],
    },
  },
  onError(ctx) {
    ctx.state.started = started;
    ctx.state.finished = finished;
  },
  onFinish() {
    release();
  },
};
