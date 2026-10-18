config.onRequest = (ctx) => ctx.reject(403);
config.providers.p.pipeline = [
  {
    name: "unused",
    onErrorResponse(ctx) {
      ctx.state.routed = true;
    },
  },
];
config.onErrorResponse = (ctx) => {
  if (ctx.error.code !== "request_rejected" || ctx.meta.provider !== null) throw Error();
  ctx.status = 422;
  ctx.body = JSON.stringify({ error: { code: "tenant_required" } });
};
