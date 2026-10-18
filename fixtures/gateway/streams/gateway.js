import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    ctx.state.explicitName = ctx.body.explicitName === true;
  });
  const provider0 = gateway.provider("p", { endpoint });
  provider0.use({
    name: "filter-and-split",
    onUpstreamEvent(ctx) {
      const content = ctx.event.choices[0]?.delta.content;
      if (content === "remove") ctx.replaceEvents([]);
      if (content === "hello") {
        const left = JSON.parse(JSON.stringify(ctx.event));
        const right = JSON.parse(JSON.stringify(ctx.event));
        left.choices[0].delta.content = "he";
        right.choices[0].delta.content = "llo";
        ctx.replaceEvents([left, right]);
      }
    },
  });
  provider0.use({
    name: "normalize",
    async onUpstreamEvent(ctx) {
      await Promise.resolve();
      const delta = ctx.event.choices[0]?.delta;
      if (typeof delta?.content === "string") {
        delta.content = delta.content.toUpperCase();
      }
    },
    onStreamEvent(ctx) {
      const delta = ctx.event.choices[0]?.delta;
      if (typeof delta?.content === "string") delta.content = "[" + delta.content;
    },
    onResponse(ctx) {
      ctx.body.choices[0].message.content = "[" + ctx.body.choices[0].message.content;
    },
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
  model0.use({
    name: "decorate",
    async onUpstreamEvent(ctx) {
      await Promise.resolve();
      const delta = ctx.event.choices[0]?.delta;
      if (typeof delta?.content === "string") delta.content += "!";
    },
    async onStreamEvent(ctx) {
      await Promise.resolve();
      const delta = ctx.event.choices[0]?.delta;
      if (typeof delta?.content === "string") delta.content += "]";
      if (ctx.state.explicitName) ctx.event.model = "stream-model";
    },
    onResponse(ctx) {
      ctx.body.choices[0].message.content += "]";
      ctx.body.model = "completed-model";
    },
  });
});
