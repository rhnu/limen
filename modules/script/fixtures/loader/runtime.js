import { env, requiredEnv } from "limen:runtime";
import * as runtime from "limen:runtime";
import { env as reexported, requiredEnv as requiredReexported } from "./runtime-reexport.mjs";

const dynamic = await import("limen:runtime");
export default () => ({
  same:
    runtime === dynamic &&
    env === dynamic.env &&
    env === reexported &&
    requiredEnv === dynamic.requiredEnv &&
    requiredEnv === requiredReexported,
  onRequest(ctx) {
    const rejected = (run, type) => {
      try {
        run();
      } catch (error) {
        if (!(error instanceof type)) throw new Error("wrong error type");
        return;
      }
      throw new Error("runtime operation unexpectedly succeeded");
    };
    for (const read of [env, requiredEnv]) {
      rejected(() => read("A=B"), TypeError);
      rejected(() => read("x".repeat(1025)), RangeError);
    }
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
      delete runtime.requiredEnv;
    }, TypeError);
    rejected(() => {
      runtime.extra = true;
    }, TypeError);
    ctx.state.checked =
      env === reexported &&
      env === dynamic.env &&
      requiredEnv === requiredReexported &&
      requiredEnv === dynamic.requiredEnv;
  },
});
