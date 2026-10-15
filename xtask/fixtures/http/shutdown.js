import config from "./pending.js";

const onFinish = config.onFinish;
config.onFinish = (ctx) => {
  onFinish(ctx);
  if (ctx.outcome === "cancelled") throw new Error("observer failure");
};

export default config;
