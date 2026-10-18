import { env } from "limen:runtime";
import * as runtime from "limen:runtime";
import { env as reexported } from "./runtime-reexport.mjs";

const dynamic = await import("limen:runtime");
export default () => ({
  same: runtime === dynamic && env === dynamic.env && env === reexported,
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
    rejected(() => env("A=B"), TypeError);
    rejected(() => env("x".repeat(1025)), RangeError);
    rejected(() => {
      runtime.env = () => "changed";
    }, TypeError);
    rejected(() => {
      delete runtime.env;
    }, TypeError);
    rejected(() => {
      runtime.extra = true;
    }, TypeError);
    ctx.state.checked = env === reexported && env === dynamic.env;
  },
});
