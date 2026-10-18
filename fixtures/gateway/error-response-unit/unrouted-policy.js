gateway.onRequest((ctx) => ctx.reject(403));
provider.use({
  name: "unused",
  onErrorResponse(ctx) {
    ctx.state.routed = true;
  },
});
gateway.onErrorResponse((ctx) => {
  if (ctx.error.code !== "request_rejected" || ctx.meta.provider !== null) throw Error();
  ctx.status = 422;
  ctx.body = JSON.stringify({ error: { code: "tenant_required" } });
});
