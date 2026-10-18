import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    ctx.observe.record("accepted");
    if (ctx.meta.phase === "onRequest") {
      ctx.body.extension.custom.push("kept");
      ctx.reject(403);
    } else if (ctx.meta.phase === "beforeSend") {
      ctx.url = "https://fictional.invalid/changed";
      ctx.headers.append("x-owned", "value");
      ctx.reject(422);
    } else if (ctx.meta.phase === "afterReceive") ctx.status = 201;
    else if (ctx.meta.phase === "onErrorResponse") {
      ctx.status = 503;
      ctx.body = null;
      ctx.contentType = "text/plain";
    } else if (
      ctx.meta.phase === "onContent" ||
      ctx.meta.phase === "onUpstreamEvent" ||
      ctx.meta.phase === "onStreamEvent"
    )
      ctx.replaceEvents([ctx.event, { extra: "open" }]);
    if (ctx.meta.phase === "onError" || ctx.meta.phase === "onFinish") {
      if (!Object.isFrozen(ctx.state)) throw Error();
    } else ctx.state.committed = true;
  });
});
