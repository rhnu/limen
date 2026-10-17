(/** @param {import("../internal/types").Intrinsics} i */
function (i) {
  "use strict";
  const { create, descriptor, isProxy, isError, proto, errorStack,
    errorPrototypes, errorNames, wellFormed } = i;
  /** @param {unknown} value */
  function oom(value) {
    if (value === null) return true;
    if (typeof value !== "object" || isProxy(value)) return false;
    const d = descriptor(value, "message");
    return !!d && !!descriptor(d, "value") && d.value === "out of memory";
  }
  /** @param {unknown} value */
  function exception(value) {
    /** @type {{ kind: string, stack: string | null }} */
    const info = create(null);
    info.kind = "ThrownValue";
    info.stack = null;
    if (typeof value !== "object" || value === null || isProxy(value) || !isError(value)) return info;
    info.kind = "Error";
    let parent = proto(value);
    for (let depth = 0; parent !== null && depth < 8 && !isProxy(parent); depth++) {
      for (let index = 0; index < errorPrototypes.length; index++) {
        if (parent === errorPrototypes[index]) {
          info.kind = /** @type {string} */ (errorNames[index]);
          parent = null;
          break;
        }
      }
      if (parent !== null) parent = proto(parent);
    }
    const stack = errorStack(value);
    // Only this bounded, temporary string reaches the parser on the VM thread.
    if (typeof stack === "string" && stack.length <= 4096 && wellFormed(stack)) info.stack = stack;
    return info;
  }
  return { oom, exception };
})
