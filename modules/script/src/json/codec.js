(/**
 * @param {import("../internal/types").Intrinsics} i
 * @param {number} maxDepth @param {number} maxNodes @param {number} maxBytes
 * @returns {import("../internal/types").Encode}
 */
function (i, maxDepth, maxNodes, maxBytes) {
  "use strict";
  const { proto, descriptor, ownKeys, create, setProto, plain, arrayProto, isArray,
    stringify, finite, safe, integer, toNumber, testIndex, isProxy, fail: invalid,
    read: data, weakGet, newSet, setHas, setAdd, setDelete } = i;
  /** @returns {never} */
  function fail() { return invalid(); }
  /** @param {unknown} value */
  function object(value) {
    return value !== null && typeof value === "object" && !isProxy(value)
      && (proto(value) === plain || proto(value) === null);
  }
  return function encode(root, slots) {
    let nodes = 0, bytes = 0;
    const seen = newSet();
    /** @param {number} n */
    function account(n) { bytes += n; if (bytes > maxBytes) fail(); }
    /** @param {unknown} value @param {number} depth @returns {unknown} */
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
      /** @type {import("../internal/types").RecordValue} */
      const result = array ? setProto([], null) : create(null);
      const keys = ownKeys(value);
      if (keys.length > maxNodes) fail();
      if (array && /** @type {unknown[]} */ (value).length > maxNodes) fail();
      const fields = slots && weakGet(slots, value);
      if (array) {
        for (let i = 0; i < /** @type {unknown[]} */ (value).length; i++) result[i] = walk(data(value, i), depth + 1);
      }
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i];
        if (typeof key !== "string") fail();
        if (array && key === "length") continue;
        const d = descriptor(value, key);
        if (!d || !descriptor(d, "value") || !d.enumerable) fail();
        if (array) {
          // Array properties outside its indexed JSON elements are not silently lost.
          if (!testIndex(key) || toNumber(key) >= /** @type {unknown[]} */ (value).length) fail();
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
    if (typeof json !== "string") fail();
    if (json.length > maxBytes) fail();
    return json;
  }
})
