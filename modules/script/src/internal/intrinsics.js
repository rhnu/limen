"use strict";
{
  /** @param {(value: unknown) => boolean} isProxy @returns {import("./types").Intrinsics} */
  function captureIntrinsics(isProxy) {
    const proto = Object.getPrototypeOf,
      descriptor = Object.getOwnPropertyDescriptor;
    const ownKeys = Reflect.ownKeys,
      define = Object.defineProperty;
    const freeze = Object.freeze,
      seal = Object.seal,
      create = Object.create;
    const setProto = Object.setPrototypeOf,
      isArray = Array.isArray;
    const apply = Reflect.apply,
      ErrorType = TypeError;
    const WeakMapType = WeakMap,
      weakGet = WeakMap.prototype.get,
      weakSet = WeakMap.prototype.set;
    const SetType = Set,
      setHas = Set.prototype.has,
      setAdd = Set.prototype.add,
      setDelete = Set.prototype.delete;
    const indexPattern = /^(0|[1-9][0-9]*)$/,
      test = RegExp.prototype.test;
    const wellFormed = String.prototype.isWellFormed;
    // QuickJS exposes native Error inspection without invoking user accessors.
    const errorConstructor = /** @type {ErrorConstructor & { isError(value: object): boolean }} */ (
      Error
    );
    const stackGetter = descriptor(Error.prototype, "stack");
    if (!stackGetter || !stackGetter.get) throw new ErrorType("missing native error stack getter");
    const errorStack = stackGetter.get;
    /** @returns {never} */
    function fail() {
      throw new ErrorType("invalid JSON, hook configuration or context operation");
    }
    /** @param {unknown} value @returns {import("./types").RecordValue} */
    function record(value) {
      if (value === null || typeof value !== "object" || isArray(value)) fail();
      return /** @type {import("./types").RecordValue} */ (value);
    }
    /** @param {unknown} value @param {PropertyKey} key @returns {unknown} */
    function read(value, key) {
      if (value === null || (typeof value !== "object" && typeof value !== "function")) fail();
      const d = descriptor(value, key);
      if (!d || !descriptor(d, "value")) fail();
      return d.value;
    }
    /** @param {unknown} value @returns {unknown} */
    function deepFreeze(value) {
      if (value !== null && typeof value === "object") {
        const keys = ownKeys(value);
        for (let i = 0; i < keys.length; i++)
          deepFreeze(read(value, /** @type {PropertyKey} */ (keys[i])));
        freeze(value);
      }
      return value;
    }
    return {
      apply,
      proto,
      descriptor,
      ownKeys,
      define,
      freeze,
      seal,
      create,
      setProto,
      plain: Object.prototype,
      arrayProto: Array.prototype,
      isArray,
      isProxy,
      stringify: JSON.stringify,
      finite: Number.isFinite,
      safe: Number.isSafeInteger,
      integer: Number.isInteger,
      toNumber: Number,
      testIndex: (value) => apply(test, indexPattern, [value]),
      isError: errorConstructor.isError,
      errorStack: (value) => apply(errorStack, value, []),
      errorPrototypes: setProto(
        [
          Error.prototype,
          EvalError.prototype,
          RangeError.prototype,
          ReferenceError.prototype,
          SyntaxError.prototype,
          TypeError.prototype,
          URIError.prototype,
          AggregateError.prototype,
          InternalError.prototype,
        ],
        null,
      ),
      errorNames: setProto(
        [
          "Error",
          "EvalError",
          "RangeError",
          "ReferenceError",
          "SyntaxError",
          "TypeError",
          "URIError",
          "AggregateError",
          "InternalError",
        ],
        null,
      ),
      wellFormed: (value) => apply(wellFormed, value, []),
      newWeakMap: () => new WeakMapType(),
      weakGet: (map, key) => apply(weakGet, map, [key]),
      weakSet: (map, key, value) => {
        apply(weakSet, map, [key, value]);
      },
      newSet: () => new SetType(),
      setHas: (set, value) => apply(setHas, set, [value]),
      setAdd: (set, value) => {
        apply(setAdd, set, [value]);
      },
      setDelete: (set, value) => {
        apply(setDelete, set, [value]);
      },
      fail,
      read,
      record,
      deepFreeze,
    };
  }
  captureIntrinsics;
}
