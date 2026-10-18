/**
 * @template {{name: string}} Options
 * @template {string} Kind
 * @param {Options} options
 * @param {Kind} kind
 * @param {readonly Exclude<keyof Options, "name">[]} keys
 * @returns {Readonly<{name: string, builtin: Readonly<{kind: Kind} & Omit<Options, "name">>}>}
 */
export function descriptor(options, kind, keys) {
  if (
    options === null ||
    typeof options !== "object" ||
    Array.isArray(options) ||
    ![Object.prototype, null].includes(Object.getPrototypeOf(options)) ||
    Reflect.ownKeys(options).some(
      (key) => key !== "name" && !keys.some((allowed) => allowed === key),
    )
  ) {
    throw new TypeError("invalid native policy configuration");
  }
  /** @type {Record<string, unknown>} */
  const builtin = { kind };
  for (const key of keys) {
    const value = options[key];
    if (value !== undefined)
      builtin[String(key)] = Array.isArray(value) ? Object.freeze([...value]) : value;
  }
  // The selected keys are statically tied to the public factory options. Rust
  // validates their values after this plain descriptor crosses the JSON bridge.
  return Object.freeze({
    name: options.name,
    builtin: Object.freeze(/** @type {{kind: Kind} & Omit<Options, "name">} */ (builtin)),
  });
}
