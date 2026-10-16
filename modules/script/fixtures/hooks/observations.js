let started = 0, finished = 0;
const releases = [];

export default {
  onRequest(ctx) {
    const mode = ctx.meta.mode;
    if (mode === "empty") return;
    if (mode === "boundary") {
      for (let i = 0; i < 64; i++) ctx.observe.record("event_" + i);
      return;
    }
    if (mode === "no-arg") return ctx.observe.record();
    if (mode === "extra") return ctx.observe.record("first", "synthetic-private");
    if (mode === "unknown") return ctx.observe.record("synthetic-private");
    if (mode === "object") return ctx.observe.record({ toString() { throw new Error("must not run"); } });
    if (mode === "number") return ctx.observe.record(0);
    if (mode === "replace-api") { ctx.observe = {}; return; }
    if (mode === "replace-method") { ctx.observe.record = () => {}; return; }
    if (mode === "edit-function") { ctx.observe.record.extra = true; return; }
    if (mode === "edit-api") { ctx.observe.extra = true; return; }
    if (mode === "forge") { ctx.observations = { events: [0], dropped: 0 }; return; }
    if (mode === "async") return Promise.resolve().then(() => {
      ctx.observe.record("second");
      ctx.observe.record("first");
    });
    if (mode === "pending-first" || mode === "pending-second") {
      ctx.observe.record(mode === "pending-first" ? "first" : "second");
      started++;
      return new Promise(resolve => releases.push(() => {
        ctx.observe.record("third");
        finished++;
        resolve();
      }));
    }
    ctx.observe.record("first");
    if (mode === "throw") throw new Error("synthetic-private");
    if (mode === "reject") return Promise.reject(new Error("synthetic-private"));
    if (mode === "contract") return 1;
    if (mode === "async-contract") return Promise.resolve(1);
    if (mode === "invalid") { ctx.state.invalid = undefined; return; }
    if (mode === "success") {
      const property = Object.getOwnPropertyDescriptor(ctx, "observe");
      if (property.enumerable || property.writable || property.configurable
          || !Object.isFrozen(ctx.observe) || !Object.isFrozen(ctx.observe.record)) throw new Error("bad API");
      if (ctx.observe.record("first") !== undefined) throw new Error("bad return");
      ctx.observe.record("second");
      ctx.observe.record("third");
      ctx.observe.record("third");
      ctx.observe.record("first");
    }
  },
  onError(ctx) {
    ctx.started = started;
    ctx.finished = finished;
  },
  onFinish(ctx) {
    if (ctx.forge) { ctx.observations = { events: [0], dropped: 0 }; return; }
    while (releases.length) releases.shift()();
  },
};
