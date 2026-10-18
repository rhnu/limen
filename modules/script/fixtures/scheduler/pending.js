import { defineGateway } from "limen:gateway";
let started = 0;
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    if (ctx.state.pending) {
      started++;
      return new Promise(() => {});
    }
    ctx.state.done = true;
  });
  gateway.onError((ctx) => {
    ctx.state.started = started;
  });
});
