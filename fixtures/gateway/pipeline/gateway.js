import { defineGateway } from "limen:gateway";
import { endpoint } from "../endpoint.js";

export default defineGateway((gateway) => {
  gateway.onRequest(function (ctx) {
    ctx.body.model = "chat";
    ctx.state.suffix = ctx.body.suffix || "";
  });
  const provider0 = gateway.provider("p", {
    endpoint,
    auth: { bearer: "synthetic-upstream-secret" },
  });
  provider0.use({
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
  });
  const model0 = provider0.model("chat", { upstream: "real-model" });
  model0.use({
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
  });
});
