({
  name: "provider",
  onUpstreamEvent(ctx) {
    if (!Object.isFrozen(ctx.sse)) throw new Error("metadata not frozen");
    let rejected = false;
    try {
      ctx.sse.id = "changed";
    } catch {
      rejected = true;
    }
    if (!rejected) throw new Error("metadata writable");
    ctx.state.order = ["provider-upstream"];
    ctx.state.metadata = ctx.sse;
  },
  onStreamEvent(ctx) {
    ctx.state.order.push("provider-stream");
    ctx.state.streamMetadata = ctx.sse;
  },
});
