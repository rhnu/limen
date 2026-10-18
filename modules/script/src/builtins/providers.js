// Presets only compose the existing startup API. Transport, defaults precedence,
// JSON limits and final URL/header validation remain owned by the host.
const ownKeys = Reflect.ownKeys;
const descriptor = Object.getOwnPropertyDescriptor;
const prototype = Object.getPrototypeOf;
const plain = Object.prototype;
const arrayPrototype = Array.prototype;
const create = Object.create;
const freeze = Object.freeze;
const isArray = Array.isArray;
const safeInteger = Number.isSafeInteger;
const ErrorType = TypeError;

/** @typedef {import("../../../../sdk/limen").JsonObject} JsonObject */
/** @typedef {Record<string, unknown>} Fields */
/** @typedef {(value: unknown) => Fields} Lower */

/** @returns {never} */
function invalid() {
  throw new ErrorType("invalid builtin provider configuration");
}

/** Inspect data descriptors before reading values; never invoke a configuration getter.
 * @param {unknown} value @param {readonly string[]} [allowed] @returns {Fields}
 */
function fields(value, allowed) {
  if (value === null || typeof value !== "object") invalid();
  const proto = prototype(value);
  if (proto !== plain && proto !== null) invalid();
  /** @type {Fields} */
  const result = create(null);
  for (const key of ownKeys(value)) {
    if (typeof key !== "string" || (allowed && !allowed.includes(key))) invalid();
    const field = descriptor(value, key);
    if (!field || !descriptor(field, "value") || !field.enumerable) invalid();
    if (allowed && field.value === undefined) invalid();
    result[key] = field.value;
  }
  return result;
}

/** @param {unknown} value @param {readonly string[]} choices @returns {string} */
function choice(value, choices) {
  if (typeof value !== "string" || !choices.includes(value)) invalid();
  return value;
}
/** @param {unknown} value @returns {boolean} */
function boolean(value) {
  if (typeof value !== "boolean") invalid();
  return value;
}
/** @param {unknown} value @returns {string} */
function string(value) {
  if (typeof value !== "string") invalid();
  return value;
}
/** Snapshot a plain, dense array without reading index accessors.
 * @param {unknown} value @returns {string[]}
 */
function strings(value) {
  if (!isArray(value) || prototype(value) !== arrayPrototype) invalid();
  const length = descriptor(value, "length")?.value;
  if (!safeInteger(length) || length < 0 || ownKeys(value).length !== length + 1) invalid();
  /** @type {string[]} */
  const result = [];
  for (let index = 0; index < length; index++) {
    const field = descriptor(value, String(index));
    if (!field || !descriptor(field, "value") || !field.enumerable) invalid();
    result[index] = string(field.value);
  }
  return result;
}

const efforts = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
/** @param {unknown} value @returns {Fields} */
function openAI(value) {
  const options = fields(value, [
    "reasoningEffort",
    "verbosity",
    "serviceTier",
    "store",
    "parallelToolCalls",
  ]);
  /** @type {Fields} */
  const result = create(null);
  for (const key of ownKeys(options)) {
    const value = options[/** @type {string} */ (key)];
    switch (key) {
      case "reasoningEffort":
        result.reasoning_effort = choice(value, efforts);
        break;
      case "verbosity":
        result.verbosity = choice(value, ["low", "medium", "high"]);
        break;
      case "serviceTier":
        result.service_tier = choice(value, [
          "auto",
          "default",
          "flex",
          "scale",
          "priority",
          "fast",
        ]);
        break;
      case "store":
        result.store = boolean(value);
        break;
      case "parallelToolCalls":
        result.parallel_tool_calls = boolean(value);
        break;
    }
  }
  return result;
}
/** @param {unknown} value @returns {Fields} */
function deepSeek(value) {
  const options = fields(value, ["thinking", "reasoningEffort"]);
  /** @type {Fields} */
  const result = create(null);
  if ("thinking" in options)
    result.thinking = { type: choice(options.thinking, ["enabled", "disabled"]) };
  if ("reasoningEffort" in options)
    result.reasoning_effort = choice(options.reasoningEffort, efforts);
  return result;
}
/** @param {unknown} value @returns {Fields} */
function routing(value) {
  const options = fields(value, [
    "order",
    "only",
    "ignore",
    "allowFallbacks",
    "requireParameters",
    "zdr",
    "dataCollection",
    "sort",
  ]);
  /** @type {Fields} */
  const result = create(null);
  for (const key of ownKeys(options)) {
    const value = options[/** @type {string} */ (key)];
    switch (key) {
      case "order":
      case "only":
      case "ignore":
        result[key] = strings(value);
        break;
      case "allowFallbacks":
        result.allow_fallbacks = boolean(value);
        break;
      case "requireParameters":
        result.require_parameters = boolean(value);
        break;
      case "zdr":
        result.zdr = boolean(value);
        break;
      case "dataCollection":
        result.data_collection = choice(value, ["allow", "deny"]);
        break;
      case "sort":
        result.sort = choice(value, ["price", "throughput", "latency"]);
        break;
    }
  }
  return result;
}
/** @param {unknown} value @returns {Fields} */
function reasoning(value) {
  const options = fields(value, ["effort", "maxTokens", "exclude", "enabled"]);
  if ("effort" in options && "maxTokens" in options) invalid();
  /** @type {Fields} */
  const result = create(null);
  if ("effort" in options) result.effort = choice(options.effort, efforts);
  if ("maxTokens" in options) {
    const max = options.maxTokens;
    if (typeof max !== "number" || !safeInteger(max) || max < 1) invalid();
    result.max_tokens = max;
  }
  if ("exclude" in options) result.exclude = boolean(options.exclude);
  if ("enabled" in options) result.enabled = boolean(options.enabled);
  return result;
}
/** @param {unknown} value @returns {Fields} */
function openRouter(value) {
  const options = fields(value, ["provider", "reasoning"]);
  /** @type {Fields} */
  const result = create(null);
  if ("provider" in options) result.provider = routing(options.provider);
  if ("reasoning" in options) result.reasoning = reasoning(options.reasoning);
  return result;
}

/** Keep raw JSON validation with the host; only inspect the shallow merge boundary.
 * @param {Fields} config @param {Lower} lower @returns {Fields | undefined}
 */
function defaults(config, lower) {
  if (!("defaults" in config) && !("options" in config)) return undefined;
  const raw = "defaults" in config ? fields(config.defaults) : create(null);
  const typed = "options" in config ? lower(config.options) : create(null);
  for (const key of ownKeys(typed)) {
    if (descriptor(raw, key)) invalid();
    raw[/** @type {string} */ (key)] = typed[key];
  }
  return raw;
}

/** @template {JsonObject} State @template Options
 * @param {import("../../../../sdk/limen").GatewayScope<State>} gateway
 * @param {string} id @param {unknown} value @param {string} endpoint
 * @param {readonly (readonly [string, string])[]} dedicated @param {Lower} lower
 * @returns {import("../../../../sdk/limen").ProviderScope<State, Options>}
 */
function provider(gateway, id, value, endpoint, dedicated, lower) {
  const config = fields(value, [
    "apiKey",
    "endpoint",
    "headers",
    "defaults",
    "options",
    ...dedicated.map(([key]) => key),
  ]);
  const key = string(config.apiKey);
  if (key.length === 0) invalid();
  const headers = "headers" in config ? fields(config.headers) : create(null);
  const reserved = [
    "authorization",
    ...dedicated.filter(([key]) => key in config).map(([, header]) => header.toLowerCase()),
  ];
  for (const key of ownKeys(headers)) {
    if (reserved.includes(/** @type {string} */ (key).toLowerCase())) invalid();
  }
  for (const [key, header] of dedicated) {
    if (key in config) headers[header] = string(config[key]);
  }
  const requestDefaults = defaults(config, lower);
  const scope = gateway.provider(id, {
    endpoint: /** @type {string} */ ("endpoint" in config ? config.endpoint : endpoint),
    auth: { bearer: key },
    ...("headers" in config || ownKeys(headers).length > 0
      ? { headers: /** @type {Record<string, string>} */ (headers) }
      : {}),
    ...(requestDefaults === undefined
      ? {}
      : { defaults: /** @type {import("../../../../sdk/limen").Defaults} */ (requestDefaults) }),
  });
  return freeze({
    use: scope.use,
    step: scope.step,
    /** @param {string} alias @param {import("../../../../sdk/limen").ModelOptions<Options>} value */
    model(alias, value) {
      const config = fields(value, ["upstream", "defaults", "options"]);
      const requestDefaults = defaults(config, lower);
      return scope.model(alias, {
        upstream: /** @type {string} */ (config.upstream),
        ...(requestDefaults === undefined
          ? {}
          : {
              defaults: /** @type {import("../../../../sdk/limen").Defaults} */ (requestDefaults),
            }),
      });
    },
  });
}

/** Register an explicit OpenAI preset; model options lower through the same shallow defaults path.
 * @template {JsonObject} [State=JsonObject]
 * @param {import("../../../../sdk/limen").GatewayScope<State>} gateway
 * @param {string} id @param {import("limen:providers").OpenAIProviderOptions} config
 * @returns {import("../../../../sdk/limen").ProviderScope<State, import("limen:providers").OpenAIOptions>}
 */
export function createOpenAI(gateway, id, config) {
  return provider(
    gateway,
    id,
    config,
    "https://api.openai.com/v1/chat/completions",
    [
      ["organization", "OpenAI-Organization"],
      ["project", "OpenAI-Project"],
    ],
    openAI,
  );
}
/** Register DeepSeek without reading process environment or injecting dynamic request hooks.
 * @template {JsonObject} [State=JsonObject]
 * @param {import("../../../../sdk/limen").GatewayScope<State>} gateway
 * @param {string} id @param {import("limen:providers").DeepSeekProviderOptions} config
 * @returns {import("../../../../sdk/limen").ProviderScope<State, import("limen:providers").DeepSeekOptions>}
 */
export function createDeepSeek(gateway, id, config) {
  return provider(gateway, id, config, "https://api.deepseek.com/chat/completions", [], deepSeek);
}
/** Register OpenRouter with optional application attribution and typed upstream routing defaults.
 * @template {JsonObject} [State=JsonObject]
 * @param {import("../../../../sdk/limen").GatewayScope<State>} gateway
 * @param {string} id @param {import("limen:providers").OpenRouterProviderOptions} config
 * @returns {import("../../../../sdk/limen").ProviderScope<State, import("limen:providers").OpenRouterOptions>}
 */
export function createOpenRouter(gateway, id, config) {
  return provider(
    gateway,
    id,
    config,
    "https://openrouter.ai/api/v1/chat/completions",
    [
      ["appUrl", "HTTP-Referer"],
      ["appTitle", "X-OpenRouter-Title"],
    ],
    openRouter,
  );
}
