(/**
 * @param {import("./internal/types").Intrinsics} i
 * @param {import("./internal/types").ContextDependencies} capabilities
 */
function (i, capabilities) {
  "use strict";
  const { descriptor, define, seal, create, read, record, deepFreeze, fail: invalid, ownKeys, weakGet, weakSet } = i;
  /** @returns {never} */
  function fail() { return invalid(); }
  /** @typedef {{keys: (string | symbol)[], headers: import("./internal/types").HeaderDraft | undefined,
   * client: unknown, observation: import("./internal/types").ObservationDraft,
   * rejection: import("./internal/types").RejectionDraft | undefined,
   * events: import("./internal/types").EventDraft | undefined}} Prepared */
  /** @type {WeakMap<object, Prepared>} */
  const prepared = i.newWeakMap();
  /** @param {unknown} value @param {unknown} phase */
  function prepare(value, phase) {
    const root = record(value);
    if (descriptor(root, "reject") || descriptor(root, "rejection")
        || descriptor(root, "observe") || descriptor(root, "observations")) fail();
    const meta = record(read(root, "meta"));
    if (read(meta, "phase") !== phase) fail();
    const policy = capabilities.phase(phase);
    record(read(root, "state"));
    const keys = ownKeys(root);
    /** @type {Prepared} */
    const info = {
      keys, headers: undefined, client: undefined,
      observation: capabilities.observations(),
      rejection: policy.reject ? capabilities.rejection() : undefined,
      events: policy.eventHook ? capabilities.events(root) : undefined,
    };
    for (let index = 0; index < keys.length; index++) {
      const key = keys[index];
      if (key === undefined) fail();
      let value = read(root, key);
      const writable = policy.writable(key);
      if (key === "headers") {
        info.headers = capabilities.headers(value, policy.headers);
        value = info.headers.api;
      } else if (key === "client") {
        const client = record(value);
        info.client = deepFreeze(client);
        value = i.freeze({ headers: capabilities.headers(read(client, "headers"), false, true).api });
      } else if (!writable && (key !== "state" || policy.observer)) {
        deepFreeze(value);
      }
      define(root, key, { value, enumerable: true, writable, configurable: false });
    }
    if (info.rejection) define(root, "reject", { value: info.rejection.reject });
    define(root, "observe", { value: info.observation.api });
    if (info.events) define(root, "replaceEvents", { value: info.events.replace });
    seal(root);
    weakSet(prepared, root, info);
    return root;
  }
  /** @param {unknown} value */
  function project(value) {
    const root = record(value);
    const info = weakGet(prepared, root);
    if (!info) fail();
    /** @type {import("./internal/types").RecordValue} */
    const output = create(null);
    for (let index = 0; index < info.keys.length; index++) {
      const key = info.keys[index];
      if (key === undefined) fail();
      if (info.events && key === "event") continue;
      output[key] = key === "headers" && info.headers ? info.headers.snapshot() : key === "client" ? info.client : read(root, key);
    }
    const rejection = info.rejection && info.rejection.snapshot();
    if (rejection) output.rejection = rejection;
    if (info.events) output.events = info.events.snapshot();
    return { context: output, observations: info.observation.snapshot() };
  }
  return { prepare, project };
})
