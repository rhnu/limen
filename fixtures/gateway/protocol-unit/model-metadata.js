({
  name: "model",
  onUpstreamEvent(ctx) {
    ctx.state.order.push("model-upstream");
  },
  onStreamEvent(ctx) {
    ctx.state.order.push("model-stream");
  },
});
