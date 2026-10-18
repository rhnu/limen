import { descriptor } from "./internal/descriptor.js";

/** @typedef {import("../../../../sdk/limen").ContentEvent} ContentEvent */
/** @typedef {import("../../../../sdk/limen").JsonObject} JsonObject */

/** @returns {never} */
function invalid() {
  throw new TypeError("invalid content policy configuration");
}
/** @returns {never} */
function failed() {
  throw new Error("invalid content policy input");
}
/** @param {unknown} value @returns {number} */
function positive(value) {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) invalid();
  return value;
}
/** @param {unknown} value @returns {string} */
function name(value) {
  if (
    typeof value !== "string" ||
    value.length === 0 ||
    value.length > 128 ||
    /[^A-Za-z0-9._/-]/.test(value)
  )
    invalid();
  return value;
}
/** @param {JsonObject} state @param {string} key @returns {JsonObject[]} */
function parts(state, key) {
  const value = state[key];
  if (value === undefined) {
    state[key] = [];
    return /** @type {JsonObject[]} */ (state[key]);
  }
  if (!Array.isArray(value)) failed();
  return /** @type {JsonObject[]} */ (value);
}
/** @param {JsonObject[]} entries @param {{choice:number,part:string}} event */
function find(entries, event) {
  return entries.findIndex((entry) => entry.choice === event.choice && entry.part === event.part);
}

/**
 * @template {JsonObject} [State=JsonObject]
 * @param {import("limen:content").MapPartTextOptions} options
 * @returns {import("../../../../sdk/limen").ScriptStep<State>}
 */
export function mapPartText(options) {
  if (options === null || typeof options !== "object") invalid();
  const step = name(options.name);
  const bound = positive(options.maxBufferedChars);
  const maxParts = positive(options.maxParts ?? 64);
  const key = options.stateKey === undefined ? `content/map/${step}` : name(options.stateKey);
  if (!Array.isArray(options.kinds) || options.kinds.length === 0) invalid();
  const kinds = Object.freeze([...options.kinds]);
  if (
    new Set(kinds).size !== kinds.length ||
    kinds.some((kind) => !["text", "reasoning", "refusal", "tool"].includes(kind))
  )
    invalid();
  const transform = options.transform;
  if (typeof transform !== "function") invalid();
  return Object.freeze({
    name: step,
    /** @param {import("../../../../sdk/limen").ContentContext<State>} ctx */
    onContent(ctx) {
      const event = ctx.event;
      const entries = parts(ctx.state, key);
      if (event.type === "part_start" && kinds.includes(event.kind)) {
        if (entries.length >= maxParts || find(entries, event) !== -1) failed();
        entries.push({ choice: event.choice, part: event.part, kind: event.kind, text: "" });
      } else if (event.type === "part_delta" || event.type === "part_end") {
        const index = find(entries, event);
        if (index === -1) return;
        const entry = /** @type {JsonObject} */ (entries[index]);
        const text = /** @type {string} */ (entry.text);
        if (event.type === "part_delta") {
          if (text.length + event.text.length > bound) failed();
          entry.text = text + event.text;
          ctx.replaceEvents(event.tool === undefined ? [] : [{ ...event, text: "" }]);
        } else {
          const info = {
            choice: event.choice,
            part: event.part,
            kind: /** @type {import("limen:content").PartTextKind} */ (entry.kind),
            ...(event.finishReason === undefined ? {} : { finishReason: event.finishReason }),
          };
          const mapped = transform(text, Object.freeze(info));
          if (typeof mapped !== "string") failed();
          entries.splice(index, 1);
          ctx.replaceEvents(
            mapped === ""
              ? [event]
              : [
                  { type: "part_delta", choice: event.choice, part: event.part, text: mapped },
                  event,
                ],
          );
        }
      } else if (event.type === "response_end") {
        if (entries.length !== 0) failed();
        delete ctx.state[key];
      }
    },
  });
}

/** @param {import("limen:content").ReplaceTextOptions} options */
export function replaceText(options) {
  return descriptor(options, "replaceText", [
    "search",
    "replacement",
    "maxBufferedChars",
    "maxParts",
  ]);
}
/** @param {import("limen:content").SyntheticTextToolOptions} options */
export function syntheticTextTool(options) {
  return descriptor(options, "syntheticTextTool", [
    "toolName",
    "maxBufferedChars",
    "maxParts",
    "maxPendingEvents",
  ]);
}
