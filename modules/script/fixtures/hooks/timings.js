import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    if (ctx.state.mode === "throw") throw new Error("synthetic");
    if (ctx.state.mode === "invalid") ctx.state.value = undefined;
    if (ctx.state.mode === "contract") return 1;
    ctx.state.done = true;
  });
  gateway.onFinish(async (ctx) => {
    await Promise.resolve();
    if (ctx.state.mode === "throw") throw new Error("synthetic");
    if (ctx.state.mode === "invalid") ctx.state.value = undefined;
    if (ctx.state.mode === "contract") return 1;
    ctx.state.done = true;
  });
});
