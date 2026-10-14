(function (isProxy, maxDepth, maxNodes, maxBytes) {
  "use strict";
  const proto = Object.getPrototypeOf;
  const descriptor = Object.getOwnPropertyDescriptor;
  const ownKeys = Reflect.ownKeys;
  const create = Object.create;
  const setProto = Object.setPrototypeOf;
  const plain = Object.prototype;
  const arrayProto = Array.prototype;
  const isArray = Array.isArray;
  const stringify = JSON.stringify;
  const finite = Number.isFinite;
  const safe = Number.isSafeInteger;
  const integer = Number.isInteger;
  const toNumber = Number;
  const testIndex = Function.prototype.call.bind(RegExp.prototype.test, /^(0|[1-9][0-9]*)$/);
  const ErrorType = TypeError;
  const WeakMapType = WeakMap;
  const weakGet = Function.prototype.call.bind(WeakMap.prototype.get);
  const weakSet = Function.prototype.call.bind(WeakMap.prototype.set);
  const SetType = Set;
  const setHas = Function.prototype.call.bind(Set.prototype.has);
  const setAdd = Function.prototype.call.bind(Set.prototype.add);
  const setDelete = Function.prototype.call.bind(Set.prototype.delete);
  function fail() { throw new ErrorType("invalid JSON or hook configuration"); }
  function data(object, key) {
    const d = descriptor(object, key);
    if (!d || !descriptor(d, "value")) fail();
    return d.value;
  }
  function object(value) {
    return value !== null && typeof value === "object" && !isProxy(value)
      && (proto(value) === plain || proto(value) === null);
  }
  const stages = ["beforeSend", "afterReceive", "onResponse", "onUpstreamEvent", "onStreamEvent"];
  function convert(root, capture) {
    const hooks = setProto([], null);
    const slots = new WeakMapType();
    function save(owner, key) {
      if (descriptor(owner, key) === undefined) return;
      const fn = data(owner, key);
      if (typeof fn !== "function" || isProxy(fn)) fail();
      if (hooks.length >= maxNodes) fail();
      hooks[hooks.length] = fn;
      let fields = weakGet(slots, owner);
      if (!fields) { fields = create(null); weakSet(slots, owner, fields); }
      fields[key] = hooks.length;
    }
    if (capture) {
      if (!object(root)) fail();
      save(root, "onRequest"); save(root, "onError"); save(root, "onFinish");
      for (let categoryIndex = 0; categoryIndex < 2; categoryIndex++) {
        const category = categoryIndex === 0 ? "providers" : "models";
        if (descriptor(root, category) === undefined) continue;
        const group = data(root, category);
        if (!object(group)) fail();
        const names = ownKeys(group);
        if (names.length > maxNodes) fail();
        for (let i = 0; i < names.length; i++) {
          if (typeof names[i] !== "string") fail();
          const entry = data(group, names[i]);
          if (!object(entry)) fail();
          if (descriptor(entry, "pipeline") === undefined) continue;
          const pipeline = data(entry, "pipeline");
          if (!isArray(pipeline) || isProxy(pipeline)) fail();
          if (pipeline.length > maxNodes) fail();
          for (let j = 0; j < pipeline.length; j++) {
            const step = data(pipeline, j);
            if (!object(step)) fail();
            for (let k = 0; k < stages.length; k++) save(step, stages[k]);
          }
        }
      }
    }
    let nodes = 0, bytes = 0;
    const seen = new SetType();
    function account(n) { bytes += n; if (bytes > maxBytes) fail(); }
    function walk(value, depth) {
      if (++nodes > maxNodes || depth > maxDepth) fail();
      if (value === null || typeof value === "boolean") { account(5); return value; }
      if (typeof value === "string") { account(value.length * 6 + 2); return value; }
      if (typeof value === "number") {
        if (!finite(value) || (integer(value) && !safe(value))) fail();
        account(32); return value;
      }
      if (typeof value !== "object" || isProxy(value) || setHas(seen, value)) fail();
      const array = isArray(value);
      if (array ? proto(value) !== arrayProto : !object(value)) fail();
      setAdd(seen, value);
      const result = array ? setProto([], null) : create(null);
      const keys = ownKeys(value);
      if (keys.length > maxNodes) fail();
      if (array && value.length > maxNodes) fail();
      const fields = weakGet(slots, value);
      if (array) {
        for (let i = 0; i < value.length; i++) result[i] = walk(data(value, i), depth + 1);
      }
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (typeof key !== "string") fail();
        if (array && key === "length") continue;
        const d = descriptor(value, key);
        if (!d || !descriptor(d, "value") || !d.enumerable) fail();
        if (array) {
          // Array properties outside its indexed JSON elements are not silently lost.
          if (!testIndex(key) || toNumber(key) >= value.length) fail();
        } else {
          account(key.length * 6 + 3);
          result[key] = fields && descriptor(fields, key) ? fields[key] : walk(d.value, depth + 1);
        }
      }
      account(2);
      setDelete(seen, value);
      return result;
    }
    const json = stringify(walk(root, 0));
    if (json.length > maxBytes) fail();
    return { json, hooks };
  }
  function oom(value) {
    if (value === null) return true;
    if (typeof value !== "object" || isProxy(value)) return false;
    const d = descriptor(value, "message");
    return !!d && !!descriptor(d, "value") && d.value === "out of memory";
  }
  return { convert, oom };
})
