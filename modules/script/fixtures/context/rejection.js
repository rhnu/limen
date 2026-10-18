import { defineGateway } from "limen:gateway";
let release;
let started = false;

function mark(ctx) {
  ctx.state.before = true;
  if (ctx.meta.defaultStatus) ctx.reject();
  else ctx.reject(ctx.meta.status);
  ctx.state.after = true;
}

export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    switch (ctx.meta.operation) {
      case "noop":
        return;
      case "inspect": {
        const descriptor = Object.getOwnPropertyDescriptor(ctx, "reject");
        if (
          !Object.isFrozen(ctx.reject) ||
          descriptor.enumerable ||
          descriptor.writable ||
          descriptor.configurable ||
          "rejection" in ctx
        )
          throw new Error("reject contract");
        return ctx.reject(422);
      }
      case "replace":
        ctx.reject = () => {};
        return;
      case "method":
        ctx.reject.mutable = true;
        return;
      case "forge":
        ctx.rejection = { status: 422 };
        return;
      case "extra":
        ctx.reject(422, 400);
        return;
      case "twice":
        ctx.reject(403);
        ctx.reject(422);
        return;
      case "throw":
        mark(ctx);
        throw new Error("synthetic");
      case "return":
        mark(ctx);
        return {};
      case "invalid-draft":
        mark(ctx);
        ctx.state.invalid = undefined;
        return;
      case "async":
        return (async () => {
          await Promise.resolve();
          mark(ctx);
          await Promise.resolve();
          ctx.state.completed = true;
        })();
      case "async-throw":
        return (async () => {
          mark(ctx);
          await Promise.resolve();
          throw new Error("synthetic");
        })();
      case "async-return":
        return (async () => {
          mark(ctx);
          await Promise.resolve();
          return {};
        })();
      case "pending":
        return (async () => {
          mark(ctx);
          await new Promise((resolve) => {
            started = true;
            release = resolve;
          });
          ctx.state.late = true;
        })();
      default:
        mark(ctx);
    }
  });
  gateway.onError((ctx) => {
    ctx.state.started = started;
  });
  gateway.onFinish(() => {
    release();
  });
});
