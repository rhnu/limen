/** @returns {never} */
function invalid() {
  throw new TypeError("invalid policy configuration");
}

/** @param {unknown} value @returns {string} */
function stepName(value) {
  if (typeof value !== "string" || value.length < 1 || value.length > 128
      || /[^A-Za-z0-9._/-]/.test(value)) invalid();
  return value;
}

/** @param {unknown} value @returns {string | undefined} */
function observation(value) {
  if (value === undefined) return undefined;
  if (typeof value !== "string" || value.length < 1 || value.length > 64
      || !/^[a-z]/.test(value) || /[^a-z0-9_]/.test(value)) invalid();
  return value;
}

/** @param {unknown} value @returns {string} */
function businessHeader(value) {
  if (typeof value !== "string" || value.length === 0
      || /[^!#$%&'*+.^_`|~0-9A-Za-z-]/.test(value)) invalid();
  const name = value.toLowerCase();
  if ([
    "authorization", "proxy-authorization", "cookie", "set-cookie",
    "proxy-authenticate", "www-authenticate", "host", "content-length",
    "transfer-encoding", "connection", "content-encoding", "content-type",
    "proxy-connection", "keep-alive", "te", "trailer", "upgrade",
  ].includes(name)) invalid();
  return name;
}

/**
 * @template {import("../types/limen").JsonObject} [State=import("../types/limen").JsonObject]
 * @param {import("./index").CapCompletionTokensOptions} options
 * @returns {import("../types/limen").PipelineStep<State>}
 */
export function capCompletionTokens(options) {
  if (options === null || typeof options !== "object" || Array.isArray(options)) invalid();
  const name = stepName(options.name);
  const max = options.max;
  if (!Number.isSafeInteger(max) || max <= 0) invalid();
  const field = options.field === undefined ? "max_completion_tokens" : options.field;
  if (field !== "max_completion_tokens" && field !== "max_tokens") invalid();
  const other = field === "max_completion_tokens" ? "max_tokens" : "max_completion_tokens";
  const onChange = observation(options.onChange);
  return Object.freeze({
    name,
    /** @param {import("../types/limen").BeforeSendContext<State>} ctx */
    beforeSend(ctx) {
      if (Object.hasOwn(ctx.body, other)) return ctx.reject(422);
      if (Object.hasOwn(ctx.body, field)) {
        const value = ctx.body[field];
        if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
          return ctx.reject(422);
        }
        if (value <= max) return;
      }
      ctx.body[field] = max;
      if (onChange !== undefined) ctx.observe.record(onChange);
    },
  });
}

/**
 * @template {import("../types/limen").JsonObject} [State=import("../types/limen").JsonObject]
 * @param {import("./index").DropRequestFieldsOptions} options
 * @returns {import("../types/limen").PipelineStep<State>}
 */
export function dropRequestFields(options) {
  if (options === null || typeof options !== "object" || Array.isArray(options)) invalid();
  const name = stepName(options.name);
  if (!Array.isArray(options.fields) || options.fields.length === 0) invalid();
  const fields = Object.freeze([...options.fields]);
  const seen = new Set();
  for (const field of fields) {
    if (typeof field !== "string" || field.length === 0
        || ["model", "messages", "stream"].includes(field) || seen.has(field)) invalid();
    seen.add(field);
  }
  const onChange = observation(options.onChange);
  return Object.freeze({
    name,
    /** @param {import("../types/limen").BeforeSendContext<State>} ctx */
    beforeSend(ctx) {
      let changed = false;
      for (const field of fields) {
        if (Object.hasOwn(ctx.body, field)) {
          delete ctx.body[field];
          changed = true;
        }
      }
      if (changed && onChange !== undefined) ctx.observe.record(onChange);
    },
  });
}

/**
 * @template {import("../types/limen").JsonObject} [State=import("../types/limen").JsonObject]
 * @param {import("./index").ForwardClientHeaderOptions} options
 * @returns {import("../types/limen").PipelineStep<State>}
 */
export function forwardClientHeader(options) {
  if (options === null || typeof options !== "object" || Array.isArray(options)) invalid();
  const name = stepName(options.name);
  const from = businessHeader(options.from);
  const to = businessHeader(options.to);
  return Object.freeze({
    name,
    /** @param {import("../types/limen").BeforeSendContext<State>} ctx */
    beforeSend(ctx) {
      const values = ctx.client.headers.getAll(from);
      if (values.length > 1) return ctx.reject(422);
      const value = values[0];
      if (values.length === 1 && value !== undefined) ctx.headers.set(to, value);
    },
  });
}
