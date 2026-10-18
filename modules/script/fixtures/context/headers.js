import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    const mode = ctx.meta.mode;
    if (mode === "valid") {
      const values = ctx.headers.getAll("X-EXAMPLE");
      if (values.length !== 2 || values[0] !== "first" || values[1] !== "second") throw new Error();
      values[0] = "not committed";
      ctx.headers.append("x-Example", "third,with,commas");
      ctx.headers.set("Authorization", "synthetic");
      ctx.headers.append("authorization", "second");
      ctx.headers.delete("X-EXAMPLE");
      ctx.headers.append("X-Example", "final");
    } else if (mode === "get") {
      if (ctx.headers.getAll("Host")[0] !== "fictional.invalid") throw new Error();
    } else {
      ctx.headers[ctx.meta.operation](ctx.meta.name, ctx.meta.value);
    }
  });
});
