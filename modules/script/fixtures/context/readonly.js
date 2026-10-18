export default {
  onRequest(ctx) {
    switch (ctx.meta.operation) {
      case "meta-value":
        ctx.meta.clientStream = true;
        break;
      case "meta-ref":
        ctx.meta = {};
        break;
      case "state-ref":
        ctx.state = {};
        break;
      case "headers-ref":
        ctx.headers = [];
        break;
      case "headers-method":
        ctx.headers.set = () => {};
        break;
      case "new-field":
        ctx.extra = true;
        break;
      case "state":
        ctx.state.nested.value = 2;
        break;
      case "body":
        ctx.body.nested.value = 2;
        break;
      case "body-ref":
        ctx.body = { replacement: true };
        break;
      case "url":
        ctx.url = "https://fictional.invalid/changed";
        break;
      case "status":
        ctx.status = 201;
        break;
      case "outcome":
        ctx.outcome = "changed";
        break;
      case "stats":
        ctx.stats.outputBytes = 1;
        break;
      case "error":
        ctx.error.code = "changed";
        break;
      case "headers":
        ctx.headers.append("x-example", "changed");
        break;
      case "noop":
        if (ctx.headers && ctx.headers.getAll("x-example").length !== 2) throw new Error();
        break;
    }
  },
};
