import { endpoint } from "../endpoint.js";

export default {
  onRequest(ctx) {
    ctx.body.model = "chat";
    ctx.state.suffix = ctx.body.suffix || "";
  },
  providers: {
    p: {
      endpoint,
      auth: { bearer: "synthetic-upstream-secret" },
      pipeline: [
        {
          name: "provider",
          async beforeSend(ctx) {
            await Promise.resolve();
            ctx.body.model = "hook-model";
            ctx.headers.delete("Authorization");
            ctx.headers.append("X-Multiple", "one");
            ctx.headers.append("x-multiple", "two");
            ctx.url = ctx.url.replace("/custom/completion", "/modified");
          },
          afterReceive(ctx) {
            ctx.status = 201;
            ctx.headers.set("X-Draft", "provider");
          },
          onResponse(ctx) {
            ctx.body.choices[0].message.content = "hello " + ctx.body.choices[0].message.content;
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
          name: "model",
          async beforeSend(ctx) {
            if (ctx.body.fail) ctx.body.temperature = 99;
            if (ctx.body.fail === "throw") throw new Error("synthetic-body-secret");
            if (ctx.body.fail === "reject") {
              await Promise.reject(new Error("synthetic-body-secret"));
            }
            if (ctx.body.fail === "return") return { temperature: 99 };
            if (ctx.body.fail === "cycle") ctx.body.cycle = ctx.body;
          },
          afterReceive(ctx) {
            ctx.headers.set("X-Draft", "model");
          },
          async onResponse(ctx) {
            await Promise.resolve();
            ctx.body = {
              ...ctx.body,
              model: "explicit-alias",
              choices: [
                {
                  ...ctx.body.choices[0],
                  message: {
                    ...ctx.body.choices[0].message,
                    content: ctx.body.choices[0].message.content.toUpperCase() + ctx.state.suffix,
                  },
                },
              ],
            };
          },
        },
      ],
    },
  },
};
