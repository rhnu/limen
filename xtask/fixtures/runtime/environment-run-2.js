import { defineGateway } from "limen:gateway";
import { token, frozen } from "./token.mjs";
const unicode = await Promise.resolve(limen.env("LIMEN_PROBE_ENV_UNICODE"));
export default defineGateway((gateway) => {
  gateway.provider("p", { auth: { bearer: token } });
  gateway.onRequest(async (ctx) => {
    ctx.state.module = {
      token,
      frozen,
      unicode,
      missing: limen.env("LIMEN_PROBE_ENV_MISSING") === undefined,
      empty: limen.env("LIMEN_PROBE_ENV_EMPTY"),
    };
    await Promise.resolve();
    if (ctx.state.action === "read") {
      ctx.state.token = limen.env("LIMEN_PROBE_ENV_TOKEN");
      ctx.state.fallback = limen.env("LIMEN_PROBE_ENV_MISSING") ?? "fallback";
      return;
    }
    if (ctx.state.action === "uncaught") {
      limen.env("A=B");
      return;
    }
    const rejected = (run, type) => {
      try {
        run();
      } catch (error) {
        if (!(error instanceof type)) throw new Error("wrong error type");
        return;
      }
      throw new Error("invalid environment access succeeded");
    };
    for (const name of ["", "A=B", "A\0B"]) {
      rejected(() => limen.env(name), TypeError);
    }
    for (const name of [
      null,
      undefined,
      1,
      {},
      {
        toString() {
          throw new Error();
        },
      },
    ]) {
      rejected(() => limen.env(name), TypeError);
    }
    rejected(() => limen.env(), TypeError);
    rejected(() => limen.env("x".repeat(4097)), RangeError);
    rejected(() => limen.env("LIMEN_PROBE_ENV_LARGE"), RangeError);
    rejected(() => limen.env("LIMEN_PROBE_ENV_INVALID"), Error);
    rejected(() => {
      limen.env = () => "changed";
    }, TypeError);
    rejected(() => {
      globalThis.limen = {};
    }, TypeError);
    rejected(() => {
      delete limen.env;
    }, TypeError);
    rejected(() => {
      delete globalThis.limen;
    }, TypeError);
    rejected(() => {
      limen.extra = true;
    }, TypeError);
    rejected(() => Object.setPrototypeOf(limen, {}), TypeError);
    ctx.state.checked = true;
  });
});
