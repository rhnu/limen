(function (validateHeader, maxNodes, maxBytes, maxEvents, observationNames, maxObservations) {
  "use strict";
  // Capture intrinsics before trusted configuration can replace global methods.
  const ownKeys = Reflect.ownKeys;
  const descriptor = Object.getOwnPropertyDescriptor;
  const define = Object.defineProperty;
  const freeze = Object.freeze;
  const seal = Object.seal;
  const create = Object.create;
  const setProto = Object.setPrototypeOf;
  const isArray = Array.isArray;
  const ErrorType = TypeError;
  const WeakMapType = WeakMap;
  const weakGet = Function.prototype.call.bind(WeakMap.prototype.get);
  const weakSet = Function.prototype.call.bind(WeakMap.prototype.set);
  const prepared = new WeakMapType();
  const observationIds = create(null);
  for (let i = 0; i < observationNames.length; i++) observationIds[observationNames[i]] = i;
  freeze(observationIds);
  function fail() { throw new ErrorType("invalid hook context or header operation"); }
  function read(object, key) {
    const d = descriptor(object, key);
    if (!d || !descriptor(d, "value")) fail();
    return d.value;
  }
  function deepFreeze(value) {
    if (value !== null && typeof value === "object") {
      const keys = ownKeys(value);
      for (let i = 0; i < keys.length; i++) deepFreeze(read(value, keys[i]));
      freeze(value);
    }
    return value;
  }
  function headerDraft(input, writable, readOnly = false) {
    if (!isArray(input)) fail();
    let fields = setProto([], null), bytes = 0;
    function normalized(name, value, mutation) {
      if (typeof name !== "string" || typeof value !== "string") fail();
      const normalized = validateHeader(name, value, mutation);
      if (typeof normalized !== "string") fail();
      return normalized;
    }
    function appendField(name, value) {
      const added = name.length + value.length;
      if (fields.length >= maxNodes || added + bytes > maxBytes) fail();
      fields[fields.length] = freeze({ name, value });
      bytes += added;
    }
    for (let i = 0; i < input.length; i++) {
      const field = input[i];
      appendField(normalized(read(field, "name"), read(field, "value"), false), read(field, "value"));
    }
    function remove(name) {
      const next = setProto([], null);
      let nextBytes = 0;
      for (let i = 0; i < fields.length; i++) {
        if (fields[i].name !== name) {
          next[next.length] = fields[i];
          nextBytes += fields[i].name.length + fields[i].value.length;
        }
      }
      fields = next; bytes = nextBytes;
    }
    function mutation(name, value) {
      if (!writable) fail();
      return normalized(name, value, true);
    }
    const api = {
      getAll(name) {
        name = normalized(name, "", false);
        const values = [];
        for (let i = 0; i < fields.length; i++) {
          if (fields[i].name === name) values[values.length] = fields[i].value;
        }
        return values;
      },
    };
    if (!readOnly) {
      api.append = function (name, value) { appendField(mutation(name, value), value); };
      api.set = function (name, value) {
        name = mutation(name, value);
        remove(name); appendField(name, value);
      };
      api.delete = function (name) { remove(mutation(name, "")); };
    }
    freeze(api);
    // Prevent changes to method function objects from becoming shared mutable state.
    const methods = ownKeys(api);
    for (let i = 0; i < methods.length; i++) freeze(api[methods[i]]);
    return { api, snapshot() {
      const output = [];
      for (let i = 0; i < fields.length; i++) output[output.length] = fields[i];
      return output;
    } };
  }
  function prepare(root) {
    if (descriptor(root, "reject") || descriptor(root, "rejection")
        || descriptor(root, "observe") || descriptor(root, "observations")) fail();
    // Older standalone scheduler experiments use plain JSON drafts.
    if (!descriptor(root, "meta")) return root;
    const meta = read(root, "meta");
    if (meta === null || typeof meta !== "object" || isArray(meta)) fail();
    const phase = read(meta, "phase");
    if (phase !== "onRequest" && phase !== "beforeSend" && phase !== "afterReceive"
        && phase !== "onResponse" && phase !== "onContent" && phase !== "onUpstreamEvent" && phase !== "onStreamEvent"
        && phase !== "onErrorResponse" && phase !== "onError" && phase !== "onFinish") fail();
    const observer = phase === "onError" || phase === "onFinish";
    const eventHook = phase === "onContent" || phase === "onUpstreamEvent" || phase === "onStreamEvent";
    const state = read(root, "state");
    if (state === null || typeof state !== "object" || isArray(state)) fail();
    const keys = ownKeys(root);
    let headers, client, events, rejection, replaced = false;
    const observationEvents = setProto([], null);
    let observationsDropped = 0;
    const observe = freeze({ record: freeze(function (name) {
      if (arguments.length !== 1 || typeof name !== "string") fail();
      const registration = descriptor(observationIds, name);
      if (!registration) fail();
      const id = registration.value;
      for (let i = 0; i < observationEvents.length; i++) {
        if (observationEvents[i] === id) return;
      }
      if (observationEvents.length >= maxObservations) {
        if (observationsDropped < 9007199254740991) observationsDropped++;
        return;
      }
      observationEvents[observationEvents.length] = id;
    }) });
    if (eventHook && (!descriptor(root, "event") || descriptor(root, "events") || descriptor(root, "replaceEvents"))) fail();
    for (let i = 0; i < keys.length; i++) {
      const key = keys[i];
      let value = read(root, key), writable = false;
      if (key === "headers") {
        headers = headerDraft(value, phase === "beforeSend" || phase === "afterReceive" || phase === "onErrorResponse");
        value = headers.api;
      } else if (key === "client") {
        if (value === null || typeof value !== "object" || isArray(value)) fail();
        client = deepFreeze(value);
        value = freeze({ headers: headerDraft(read(client, "headers"), false, true).api });
      } else if (key === "body" && (phase === "onRequest" || phase === "beforeSend" || phase === "onResponse" || phase === "onErrorResponse")) {
        writable = true;
      } else if (key === "event" && eventHook) {
        writable = true;
      } else if ((key === "url" && phase === "beforeSend")
          || (key === "status" && (phase === "afterReceive" || phase === "onErrorResponse"))
          || (key === "contentType" && phase === "onErrorResponse")) {
        writable = true;
      } else if (key !== "state" || observer) {
        deepFreeze(value);
      }
      define(root, key, { value, enumerable: true, writable, configurable: false });
    }
    if (phase === "onRequest" || phase === "beforeSend") {
      define(root, "reject", {
        value: freeze(function (status = 400) {
          if (arguments.length > 1 || rejection || (status !== 400 && status !== 403 && status !== 422)) fail();
          rejection = freeze({ status });
        }),
        enumerable: false, writable: false, configurable: false,
      });
    }
    define(root, "observe", { value: observe, enumerable: false, writable: false, configurable: false });
    if (eventHook) {
      define(root, "replaceEvents", {
        value: freeze(function (replacement) {
          if (replaced || !isArray(replacement) || replacement.length > maxEvents) fail();
          replaced = true;
          events = replacement;
        }),
        enumerable: false, writable: false, configurable: false,
      });
    }
    seal(root);
    weakSet(prepared, root, { keys, headers, client, eventHook, observations() {
      const events = [];
      for (let i = 0; i < observationEvents.length; i++) {
        define(events, i, { value: observationEvents[i], enumerable: true, writable: true, configurable: true });
      }
      return { events, dropped: observationsDropped };
    }, rejection() { return rejection; }, events() {
      return replaced ? events : [read(root, "event")];
    } });
    return root;
  }
  function project(root) {
    const info = weakGet(prepared, root);
    if (!info) {
      if (descriptor(root, "observe") || descriptor(root, "observations")) fail();
      return { context: root, observations: { events: [], dropped: 0 } };
    }
    const output = create(null);
    for (let i = 0; i < info.keys.length; i++) {
      const key = info.keys[i];
      if (info.eventHook && key === "event") continue;
      output[key] = key === "headers" ? info.headers.snapshot() : key === "client" ? info.client : read(root, key);
    }
    const rejection = info.rejection();
    if (rejection) output.rejection = rejection;
    if (info.eventHook) {
      const events = info.events();
      // Recheck at commit: an async hook may retain and grow the replacement array.
      if (events.length > maxEvents) fail();
      for (let i = 0; i < events.length; i++) {
        const event = read(events, i);
        if (event === null || typeof event !== "object" || isArray(event)) fail();
      }
      output.events = events;
    }
    return { context: output, observations: info.observations() };
  }
  return { prepare, project };
})
