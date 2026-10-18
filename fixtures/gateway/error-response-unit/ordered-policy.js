config.providers.p.pipeline = [
  {
    name: "provider",
    onErrorResponse(ctx) {
      ctx.state.order = ["provider"];
      ctx.status = 422;
      const body = JSON.parse(ctx.body);
      body.provider = true;
      ctx.body = JSON.stringify(body);
      ctx.headers.set("x-adapted", "yes");
      ctx.observe.record("adapted");
    },
  },
];
config.models.chat.pipeline = [
  {
    name: "model",
    onErrorResponse(ctx) {
      ctx.state.order.push("model");
      const body = JSON.parse(ctx.body);
      body.model = true;
      ctx.body = JSON.stringify(body);
    },
  },
];
config.onErrorResponse = async (ctx) => {
  await Promise.resolve();
  ctx.state.order.push("global");
  ctx.status = 409;
  ctx.contentType = "application/problem+json";
};
config.onError = (ctx) => {
  if (
    ctx.error.code !== "upstream_failed" ||
    ctx.error.status !== 409 ||
    ctx.meta.upstreamStatus !== 429 ||
    ctx.state.order.join() !== "provider,model,global"
  )
    throw Error();
  ctx.observe.record("adapted");
};
config.onFinish = (ctx) => {
  if (ctx.outcome !== "failed") throw Error();
};
