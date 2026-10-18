import { defineGateway } from "limen:gateway";
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    switch (ctx.meta.operation) {
      case "client-ref":
        ctx.client = {};
        break;
      case "headers-ref":
        ctx.client.headers = {};
        break;
      case "method-ref":
        ctx.client.headers.getAll = () => [];
        break;
      case "method-object":
        ctx.client.headers.getAll.mutable = true;
        break;
      case "new-field":
        ctx.client.mutable = true;
        break;
      case "append":
        ctx.client.headers.append("x-example", "changed");
        break;
      case "set":
        ctx.client.headers.set("x-example", "changed");
        break;
      case "delete":
        ctx.client.headers.delete("x-example");
        break;
      default: {
        const headers = ctx.client.headers;
        if (
          !Object.isFrozen(ctx.client) ||
          !Object.isFrozen(headers) ||
          !Object.isFrozen(headers.getAll) ||
          "append" in headers ||
          "set" in headers ||
          "delete" in headers
        )
          throw new Error("readonly headers");
        const values = headers.getAll("X-EXAMPLE");
        if (
          values.length !== 3 ||
          values[0] !== "first" ||
          values[1] !== "second,third" ||
          values[2] !== "" ||
          headers.getAll("x-missing").length !== 0
        )
          throw new Error("header values");
        values[0] = "changed";
        values.push("extra");
        if (headers.getAll("x-example")[0] !== "first" || headers.getAll("x-example").length !== 3)
          throw new Error("mutable snapshot");
        if (ctx.headers) ctx.headers.append("x-upstream", "independent");
        if (ctx.meta.phase !== "onError" && ctx.meta.phase !== "onFinish") ctx.state.read = true;
      }
    }
  });
});
