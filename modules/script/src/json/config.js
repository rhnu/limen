(/**
 * @param {import("../internal/types").Intrinsics} i @param {number} maxNodes
 * @param {import("../internal/types").Encode} encode
 * @returns {(root: unknown) => {json: string, hooks: import("../internal/types").Hook[]}}
 */
function (i, maxNodes, encode) {
  "use strict";
  const { proto, descriptor, ownKeys, create, setProto, plain, isArray, isProxy, fail: invalid,
    read: data, weakGet, weakSet } = i;
  /** @returns {never} */
  function fail() { return invalid(); }
  /** @param {unknown} value @returns {value is object} */
  function object(value) {
    return value !== null && typeof value === "object" && !isProxy(value)
      && (proto(value) === plain || proto(value) === null);
  }
  const stages = ["beforeSend", "afterReceive", "onResponse", "onContent", "onUpstreamEvent", "onStreamEvent", "onErrorResponse"];
  return function capture(root) {
    /** @type {import("../internal/types").Hook[]} */
    const hooks = setProto([], null);
    /** @type {WeakMap<object, import("../internal/types").RecordValue>} */
    const slots = i.newWeakMap();
    /** @param {object} owner @param {string} key */
    function save(owner, key) {
      if (descriptor(owner, key) === undefined) return;
      const fn = data(owner, key);
      if (typeof fn !== "function" || isProxy(fn)) fail();
      if (hooks.length >= maxNodes) fail();
      hooks[hooks.length] = /** @type {import("../internal/types").Hook} */ (fn);
      let fields = weakGet(slots, owner);
      if (!fields) {
        fields = /** @type {import("../internal/types").RecordValue} */ (create(null));
        weakSet(slots, owner, fields);
      }
      fields[key] = hooks.length;
    }
    if (!object(root)) fail();
    save(root, "onRequest"); save(root, "onErrorResponse"); save(root, "onError"); save(root, "onFinish");
    for (let categoryIndex = 0; categoryIndex < 2; categoryIndex++) {
      const category = categoryIndex === 0 ? "providers" : "models";
      if (descriptor(root, category) === undefined) continue;
      const group = data(root, category);
      if (!object(group)) fail();
      const names = ownKeys(group);
      if (names.length > maxNodes) fail();
      for (let i = 0; i < names.length; i++) {
        if (typeof names[i] !== "string") fail();
        const entry = data(group, /** @type {string} */ (names[i]));
        if (!object(entry)) fail();
        if (descriptor(entry, "pipeline") === undefined) continue;
        const pipeline = data(entry, "pipeline");
        if (!isArray(pipeline) || isProxy(pipeline)) fail();
        if (pipeline.length > maxNodes) fail();
        for (let j = 0; j < pipeline.length; j++) {
          const step = data(pipeline, j);
          if (!object(step)) fail();
          for (let k = 0; k < stages.length; k++) save(step, /** @type {string} */ (stages[k]));
        }
      }
    }
    return { json: encode(root, slots), hooks };
  };
})
