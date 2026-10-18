(ctx) => {
  if (
    !Object.isFrozen(ctx.meta) ||
    !Object.isFrozen(ctx.client) ||
    !Object.isFrozen(ctx.client.headers)
  )
    throw Error();
  const values = ctx.client.headers.getAll("x-original");
  values[0] = "changed";
  if (ctx.client.headers.getAll("X-Original")[0] !== "first") throw Error();
};
