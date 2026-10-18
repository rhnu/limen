import { env } from "limen:runtime";
import * as runtime from "limen:runtime";
import { defineGateway } from "limen:gateway";
import { token, sealed } from "./token.mjs";
const unicode = await Promise.resolve(env("LIMEN_PROBE_ENV_UNICODE"));
export default defineGateway((gateway) => {
  gateway.provider("p", { auth: { bearer: token } });
  gateway.onRequest(async (ctx) => {
    ctx.state.module = {
      token,
      sealed,
      unicode,
      missing: env("LIMEN_PROBE_ENV_MISSING") === undefined,
      empty: env("LIMEN_PROBE_ENV_EMPTY"),
    };
    await Promise.resolve();
    if (ctx.state.action === "read") {
      ctx.state.token = env("LIMEN_PROBE_ENV_TOKEN");
      ctx.state.fallback = env("LIMEN_PROBE_ENV_MISSING") ?? "fallback";
      return;
    }
    if (ctx.state.action === "uncaught") {
      env("A=B");
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
      rejected(() => env(name), TypeError);
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
      rejected(() => env(name), TypeError);
    }
    rejected(() => env(), TypeError);
    rejected(() => env("x".repeat(4097)), RangeError);
    rejected(() => env("LIMEN_PROBE_ENV_LARGE"), RangeError);
    rejected(() => env("LIMEN_PROBE_ENV_INVALID"), Error);
    rejected(() => {
      env = () => "changed";
    }, TypeError);
    rejected(() => {
      runtime.env = () => "changed";
    }, TypeError);
    rejected(() => {
      delete runtime.env;
    }, TypeError);
    rejected(() => {
      runtime.extra = true;
    }, TypeError);
    rejected(() => Object.setPrototypeOf(runtime, {}), TypeError);
    ctx.state.checked = true;
  });
});
