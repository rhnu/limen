import { defineGateway } from "limen:gateway";
const NativeError = Error;
const reflectSet = Reflect.set;
let touched = 0;
const trap = () => {
  touched++;
  throw new NativeError("intrinsic replacement invoked");
};
// Replace globals during module evaluation, before configuration hooks are extracted.
for (const key of [
  "getPrototypeOf",
  "getOwnPropertyDescriptor",
  "defineProperty",
  "freeze",
  "seal",
  "create",
  "setPrototypeOf",
])
  Object[key] = trap;
Reflect.ownKeys = trap;
Array.isArray = trap;
JSON.stringify = trap;
Number.isFinite = Number.isSafeInteger = Number.isInteger = trap;
WeakMap.prototype.get = WeakMap.prototype.set = trap;
Set.prototype.has = Set.prototype.add = Set.prototype.delete = trap;
RegExp.prototype.test = trap;
Function.prototype.call = trap;
export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    if (touched !== 0 || reflectSet(ctx.meta, "phase", "forged"))
      throw new NativeError("intrinsics or readonly facts changed");
    ctx.observe.record("changed");
    if (ctx.meta.phase === "beforeSend") {
      ctx.headers.set("X-Example", "isolated");
      ctx.reject(403);
      ctx.state.checked = true;
    } else if (ctx.meta.phase === "onContent") {
      ctx.replaceEvents([{ text: "captured" }]);
      ctx.state.checked = true;
    } else if (ctx.meta.phase === "onFinish" && reflectSet(ctx.state, "checked", true)) {
      throw new NativeError("observer state changed");
    }
  });
});
