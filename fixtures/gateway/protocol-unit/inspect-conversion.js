({
  name: "inspect",
  onStreamEvent(ctx) {
    if (ctx.sse !== null) throw new Error("synthetic transport metadata");
    const choice = ctx.event.choices[0];
    if (!choice) return;
    ctx.state.message ||= {};
    Object.assign(ctx.state.message, choice.delta || {});
    if ("annotations" in choice) ctx.state.annotations = choice.annotations;
    if ("logprobs" in choice) ctx.state.logprobs = choice.logprobs;
    if (choice.finish_reason) ctx.state.finish = choice.finish_reason;
  },
});
