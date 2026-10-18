import { endpoint } from "../endpoint.js";

export default {
  onRequest(ctx) {
    ctx.state.explicitName = ctx.body.explicitName === true;
  },
  providers: {
    p: {
      endpoint,
      pipeline: [
        {
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
        },
        {
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
        },
      ],
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "real-model",
      pipeline: [
        {
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
        },
      ],
    },
  },
};
