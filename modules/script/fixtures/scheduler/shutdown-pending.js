import { defineGateway } from "limen:gateway";
let started = false;
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    return new Promise(() => {
      started = true;
      ctx.state.pending = true;
    });
  });
  gateway.onError((ctx) => {
    ctx.state.started = started;
  });
});
