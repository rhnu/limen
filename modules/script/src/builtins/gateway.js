// Composition happens once at startup; only the resulting plain configuration
// crosses the private JSON/hook capture boundary.
const ownKeys = Reflect.ownKeys;
const descriptor = Object.getOwnPropertyDescriptor;
const prototype = Object.getPrototypeOf;
const plain = Object.prototype;
const create = Object.create;
const freeze = Object.freeze;
const ErrorType = TypeError;

/** @returns {never} */
function invalid() {
  throw new ErrorType("invalid gateway composition");
}

/** @param {unknown} value @returns {Record<string, unknown>} */
function record(value) {
  if (value === null || typeof value !== "object") invalid();
  if (prototype(value) !== plain && prototype(value) !== null) invalid();
  return /** @type {Record<string, unknown>} */ (value);
}

/** @param {unknown} value @param {readonly string[]} keys */
function copy(value, keys) {
  const source = record(value);
  /** @type {Record<string, unknown>} */
  const target = create(null);
  for (const key of ownKeys(source)) {
    if (typeof key !== "string") invalid();
    let allowed = false;
    for (const candidate of keys) if (candidate === key) allowed = true;
    const field = descriptor(source, key);
    if (!allowed || !field || descriptor(field, "value") === undefined) invalid();
    target[key] = field.value;
  }
  return target;
}

/** @param {unknown} value @returns {string} */
function name(value) {
  if (typeof value !== "string" || value.length === 0) invalid();
  return value;
}

/**
 * @template {import("../../../../sdk/limen").JsonObject} State
 * @param {import("../../../../sdk/limen").GatewaySetup<State>} setup
 * @returns {import("../../../../sdk/limen").GatewayFactory}
 */
export function defineGateway(setup) {
  if (typeof setup !== "function") invalid();
  return function buildGateway() {
    let open = true;
    /** @type {Record<string, unknown>} */
    const providers = create(null),
      models = create(null);
    /** @type {Record<string, unknown>} */
    const config = { providers, models };
    function check() {
      if (!open) invalid();
    }
    /** @param {Record<string, unknown>} owner @param {string} stage @param {unknown} hook @returns {undefined} */
    function register(owner, stage, hook) {
      check();
      if (descriptor(owner, stage) !== undefined || typeof hook !== "function") invalid();
      owner[stage] = hook;
    }
    /** @param {Record<string, unknown>} entry @returns {import("../../../../sdk/limen").ModelScope<State>} */
    function pipeline(entry) {
      /** @type {Record<string, unknown>[]} */
      const steps = [];
      entry.pipeline = steps;
      /** @type {Record<string, unknown>} */
      const names = create(null);
      /** @param {Record<string, unknown>} step */
      function append(step) {
        check();
        const field = descriptor(step, "name");
        if (!field || descriptor(field, "value") === undefined) invalid();
        const id = name(field.value);
        if (descriptor(names, id) !== undefined) invalid();
        names[id] = true;
        steps[steps.length] = step;
      }
      return {
        /** @param {import("../../../../sdk/limen").PipelineStep<State>} step */
        use(step) {
          check();
          // Preserve the descriptor for protected host capture, including its
          // nested values and proxy/accessor checks. Options are computed during setup.
          append(record(step));
        },
        /** @param {string} id @param {import("../../../../sdk/limen").StepSetup<State>} setupStep */
        step(id, setupStep) {
          check();
          if (typeof setupStep !== "function") invalid();
          if (descriptor(names, name(id)) !== undefined) invalid();
          /** @type {Record<string, unknown>} */
          const step = { name: name(id) };
          let stepOpen = true;
          /** @param {string} stage @param {unknown} hook @returns {undefined} */
          function add(stage, hook) {
            if (!stepOpen) invalid();
            register(step, stage, hook);
          }
          /** @type {import("../../../../sdk/limen").StepScope<State>} */
          const scope = {
            beforeSend: (hook) => add("beforeSend", hook),
            afterReceive: (hook) => add("afterReceive", hook),
            onResponse: (hook) => add("onResponse", hook),
            onContent: (hook) => add("onContent", hook),
            onUpstreamEvent: (hook) => add("onUpstreamEvent", hook),
            onStreamEvent: (hook) => add("onStreamEvent", hook),
            onErrorResponse: (hook) => add("onErrorResponse", hook),
          };
          try {
            if (setupStep(freeze(scope)) !== undefined) invalid();
            if (ownKeys(step).length === 1) invalid();
            append(step);
          } finally {
            stepOpen = false;
          }
        },
      };
    }
    /** @type {import("../../../../sdk/limen").GatewayScope<State>} */
    const gateway = {
      provider(id, options) {
        check();
        id = name(id);
        if (descriptor(providers, id) !== undefined) invalid();
        const entry = copy(options, ["endpoint", "auth", "headers", "defaults"]);
        providers[id] = entry;
        return freeze({
          ...pipeline(entry),
          /** @param {string} alias @param {import("../../../../sdk/limen").ModelOptions} options */
          model(alias, options) {
            check();
            alias = name(alias);
            if (descriptor(models, alias) !== undefined) invalid();
            const model = copy(options, ["upstream", "defaults"]);
            model.provider = id;
            models[alias] = model;
            return freeze(pipeline(model));
          },
        });
      },
      onRequest: (hook) => register(config, "onRequest", hook),
      onErrorResponse: (hook) => register(config, "onErrorResponse", hook),
      onError: (hook) => register(config, "onError", hook),
      onFinish: (hook) => register(config, "onFinish", hook),
    };
    try {
      if (setup(freeze(gateway)) !== undefined) invalid();
      return config;
    } finally {
      open = false;
    }
  };
}
