import { env, requiredEnv } from "limen:runtime";
import * as runtime from "limen:runtime";
import { defineGateway } from "limen:gateway";
import { token, sealed } from "./token.mjs";
const unicode = await Promise.resolve(requiredEnv("LIMEN_PROBE_ENV_UNICODE"));
export default defineGateway((gateway) => {
  gateway.provider("p", { auth: { bearer: token } });
  gateway.onRequest(async (ctx) => {
    ctx.state.module = {
      token,
      sealed,
      unicode,
      missing: env("LIMEN_PROBE_ENV_MISSING") === undefined,
      empty: env("LIMEN_PROBE_ENV_EMPTY"),
      whitespace: requiredEnv("LIMEN_PROBE_ENV_WHITESPACE"),
    };
    await Promise.resolve();
    if (ctx.state.action === "read") {
      ctx.state.token = requiredEnv("LIMEN_PROBE_ENV_TOKEN");
      ctx.state.fallback = env("LIMEN_PROBE_ENV_MISSING") ?? "fallback";
      return;
    }
    if (ctx.state.action === "uncaught") {
      env("A=B");
      return;
    }
    if (ctx.state.action === "required-uncaught") {
      requiredEnv("LIMEN_PROBE_ENV_MISSING");
      return;
    }
    const rejected = (run, type) => {
      try {
        run();
      } catch (error) {
        if (!(error instanceof type)) throw new Error("wrong error type");
        if (/LIMEN_PROBE_ENV_|synthetic-upstream-token/.test(error.message))
          throw new Error("environment error exposed input");
        return;
      }
      throw new Error("invalid environment access succeeded");
    };
    for (const read of [env, requiredEnv]) {
      for (const name of ["", "A=B", "A\0B"]) {
        rejected(() => read(name), TypeError);
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
        rejected(() => read(name), TypeError);
      }
      rejected(() => read(), TypeError);
      rejected(() => read("x".repeat(4097)), RangeError);
      rejected(() => read("LIMEN_PROBE_ENV_LARGE"), RangeError);
      rejected(() => read("LIMEN_PROBE_ENV_INVALID"), Error);
    }
    rejected(() => requiredEnv("LIMEN_PROBE_ENV_MISSING"), Error);
    rejected(() => requiredEnv("LIMEN_PROBE_ENV_EMPTY"), Error);
    rejected(() => {
      env = () => "changed";
    }, TypeError);
    rejected(() => {
      requiredEnv = () => "changed";
    }, TypeError);
    rejected(() => {
      runtime.env = () => "changed";
    }, TypeError);
    rejected(() => {
      delete runtime.env;
    }, TypeError);
    rejected(() => {
      runtime.requiredEnv = () => "changed";
    }, TypeError);
    rejected(() => {
      runtime.extra = true;
    }, TypeError);
    rejected(() => Object.setPrototypeOf(runtime, {}), TypeError);
    ctx.state.checked = true;
  });
});
