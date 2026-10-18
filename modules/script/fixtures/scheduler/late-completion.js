import { defineGateway } from "limen:gateway";
let release;
let changes = 0;
let started = 0;
export default defineGateway((gateway) => {
  gateway.onRequest(async (ctx) => {
    started++;
    ctx.state.draft = "uncommitted";
    await new Promise((resolve) => {
      release = resolve;
    });
    changes++;
    ctx.state.draft = "late";
  });
  gateway.onFinish(() => {
    release();
  });
  gateway.onError((ctx) => {
    ctx.state.changes = changes;
    ctx.state.started = started;
  });
});
