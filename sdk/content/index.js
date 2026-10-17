/** @typedef {import("../limen").ContentEvent} ContentEvent */
/** @typedef {import("../limen").JsonObject} JsonObject */

/** @returns {never} */
function invalid() { throw new TypeError("invalid content policy configuration"); }
/** @returns {never} */
function failed() { throw new Error("invalid content policy input"); }
/** @param {unknown} value @returns {number} */
function positive(value) {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) invalid();
  return value;
}
/** @param {unknown} value @returns {string} */
function name(value) {
  if (typeof value !== "string" || value.length === 0 || value.length > 128
      || /[^A-Za-z0-9._/-]/.test(value)) invalid();
  return value;
}
/** @param {JsonObject} state @param {string} key @returns {JsonObject[]} */
function parts(state, key) {
  const value = state[key];
  if (value === undefined) { state[key] = []; return /** @type {JsonObject[]} */ (state[key]); }
  if (!Array.isArray(value)) failed();
  return /** @type {JsonObject[]} */ (value);
}
/** @param {JsonObject[]} entries @param {{choice:number,part:string}} event */
function find(entries, event) {
  return entries.findIndex(entry => entry.choice === event.choice && entry.part === event.part);
}

/**
 * @template {JsonObject} [State=JsonObject]
 * @param {import("./index").MapPartTextOptions} options
 * @returns {import("../limen").PipelineStep<State>}
 */
export function mapPartText(options) {
  if (options === null || typeof options !== "object") invalid();
  const step = name(options.name);
  const bound = positive(options.maxBufferedChars);
  const maxParts = positive(options.maxParts ?? 64);
  const key = options.stateKey === undefined ? `content/map/${step}` : name(options.stateKey);
  if (!Array.isArray(options.kinds) || options.kinds.length === 0) invalid();
  const kinds = Object.freeze([...options.kinds]);
  if (new Set(kinds).size !== kinds.length || kinds.some(kind => !["text", "reasoning", "refusal", "tool"].includes(kind))) invalid();
  const transform = options.transform;
  if (typeof transform !== "function") invalid();
  return Object.freeze({
    name: step,
    /** @param {import("../limen").ContentContext<State>} ctx */
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
            choice: event.choice, part: event.part,
            kind: /** @type {import("./index").PartTextKind} */ (entry.kind),
            ...(event.finishReason === undefined ? {} : { finishReason: event.finishReason }),
          };
          const mapped = transform(text, Object.freeze(info));
          if (typeof mapped !== "string") failed();
          entries.splice(index, 1);
          ctx.replaceEvents(mapped === "" ? [event] : [
            { type: "part_delta", choice: event.choice, part: event.part, text: mapped }, event,
          ]);
        }
      } else if (event.type === "response_end") {
        if (entries.length !== 0) failed();
        delete ctx.state[key];
      }
    },
  });
}

/**
 * @template {JsonObject} [State=JsonObject]
 * @param {import("./index").ReplaceTextOptions} options
 * @returns {import("../limen").PipelineStep<State>}
 */
export function replaceText(options) {
  if (options === null || typeof options !== "object") invalid();
  const step = name(options.name);
  const search = options.search;
  const replacement = options.replacement;
  const bound = positive(options.maxBufferedChars);
  const maxParts = positive(options.maxParts ?? 64);
  const key = options.stateKey === undefined ? `content/replace/${step}` : name(options.stateKey);
  if (typeof search !== "string" || search.length === 0 || search.length - 1 > bound
      || typeof replacement !== "string") invalid();
  return Object.freeze({
    name: step,
    /** @param {import("../limen").ContentContext<State>} ctx */
    onContent(ctx) {
      const event = ctx.event;
      const entries = parts(ctx.state, key);
      if (event.type === "part_start" && event.kind === "text") {
        if (entries.length >= maxParts || find(entries, event) !== -1) failed();
        entries.push({ choice: event.choice, part: event.part, pending: "" });
      } else if (event.type === "part_delta" || event.type === "part_end") {
        const index = find(entries, event);
        if (index === -1) return;
        const entry = /** @type {JsonObject} */ (entries[index]);
        const pending = /** @type {string} */ (entry.pending);
        if (event.type === "part_end") {
          entries.splice(index, 1);
          ctx.replaceEvents(pending === "" ? [event] : [
            { type: "part_delta", choice: event.choice, part: event.part, text: pending }, event,
          ]);
          return;
        }
        let input = pending + event.text;
        let output = "";
        let at;
        while ((at = input.indexOf(search)) !== -1) {
          output += input.slice(0, at) + replacement;
          input = input.slice(at + search.length);
        }
        let keep = Math.min(search.length - 1, input.length);
        while (keep > 0 && !search.startsWith(input.slice(input.length - keep))) keep--;
        entry.pending = keep === 0 ? "" : input.slice(input.length - keep);
        output += input.slice(0, input.length - keep);
        ctx.replaceEvents(output === "" ? [] : [{ ...event, text: output }]);
      } else if (event.type === "response_end") {
        if (entries.length !== 0) failed();
        delete ctx.state[key];
      }
    },
  });
}

// Exact grammar: whitespace, {, "text", :, JSON string, }, whitespace.
// Keeping lexical position, one escape and one high surrogate bounds parser retention.
/** @returns {JsonObject} */
function decoder() { return { phase: 0, escape: false, hex: "", high: 0 }; }
/** @param {JsonObject} state @param {string} input @returns {string} */
function decode(state, input) {
  let output = "";
  /** @param {string} unit */
  function emit(unit) {
    const code = unit.charCodeAt(0);
    const high = /** @type {number} */ (state.high);
    if (high !== 0) {
      if (code < 0xdc00 || code > 0xdfff) failed();
      output += String.fromCharCode(high, code);
      state.high = 0;
    } else if (code >= 0xd800 && code <= 0xdbff) state.high = code;
    else if (code >= 0xdc00 && code <= 0xdfff) failed();
    else output += unit;
  }
  for (let offset = 0; offset < input.length; offset++) {
    const char = /** @type {string} */ (input[offset]);
    const phase = /** @type {number} */ (state.phase);
    if (phase === 9) {
      if (state.hex !== "") {
        if (!/[0-9a-fA-F]/.test(char)) failed();
        state.hex = /** @type {string} */ (state.hex) + char;
        if (/** @type {string} */ (state.hex).length === 5) {
          emit(String.fromCharCode(parseInt(/** @type {string} */ (state.hex).slice(1), 16)));
          state.hex = "";
        }
      } else if (state.escape) {
        state.escape = false;
        if (char === "u") state.hex = "u";
        else {
          const escapes = { '"': '"', "\\": "\\", "/": "/", b: "\b", f: "\f", n: "\n", r: "\r", t: "\t" };
          const value = /** @type {Record<string,string>} */ (escapes)[char];
          if (value === undefined) failed();
          emit(value);
        }
      } else if (char === "\\") state.escape = true;
      else if (char === '"') {
        if (state.high !== 0) failed();
        state.phase = 10;
      } else {
        if (char.charCodeAt(0) < 0x20) failed();
        emit(char);
      }
    } else if ([0, 1, 7, 8, 10, 11].includes(phase) && /[ \t\r\n]/.test(char)) {
      continue;
    } else {
      const expected = ['{', '"', 't', 'e', 'x', 't', '"', ':', '"'];
      if (phase < 9) {
        // The opening string quote is separate from the fixed property grammar.
        if (char !== expected[phase]) failed();
        state.phase = phase + 1;
      } else if (phase === 10 && char === "}") state.phase = 11;
      else failed();
    }
  }
  return output;
}

/**
 * @template {JsonObject} [State=JsonObject]
 * @param {import("./index").SyntheticTextToolOptions} options
 * @returns {import("../limen").PipelineStep<State>}
 */
export function syntheticTextTool(options) {
  if (options === null || typeof options !== "object") invalid();
  const step = name(options.name);
  const toolName = options.toolName ?? "limen_emit_text";
  if (typeof toolName !== "string" || !/^[A-Za-z_][A-Za-z0-9_-]{0,63}$/.test(toolName)) invalid();
  const bound = positive(options.maxBufferedChars);
  const maxParts = positive(options.maxParts ?? 64);
  const maxPending = positive(options.maxPendingEvents ?? 64);
  const key = options.stateKey === undefined ? `content/tool/${step}` : name(options.stateKey);
  if (bound < toolName.length) invalid();
  return Object.freeze({
    name: step,
    /** @param {import("../limen").BeforeSendContext<State>} ctx */
    beforeSend(ctx) {
      const body = ctx.body;
      if (Object.hasOwn(body, "functions") || Object.hasOwn(body, "function_call")
          || Object.hasOwn(body, "response_format") || (body.n !== undefined && body.n !== 1)
          || body.parallel_tool_calls === true
          || (body.tool_choice !== undefined && body.tool_choice !== "auto")) {
        ctx.reject(422); return;
      }
      const tools = body.tools === undefined ? [] : body.tools;
      if (!Array.isArray(tools) || tools.some(tool => {
        if (tool === null || typeof tool !== "object" || Array.isArray(tool) || tool.type !== "function") return true;
        const fn = tool.function;
        return fn === null || typeof fn !== "object" || Array.isArray(fn)
          || typeof fn.name !== "string" || fn.name === toolName;
      })) { ctx.reject(422); return; }
      body.messages.push({
        role: "user",
        content: `Transport instruction: To answer with text, call ${toolName} with exactly {"text":"your answer"}. Other declared tools remain available.`,
      });
      body.tools = [...tools, {
        type: "function",
        function: {
          name: toolName,
          description: "Return the assistant's text answer through the transport.",
          parameters: { type: "object", properties: { text: { type: "string" } }, required: ["text"], additionalProperties: false },
        },
      }];
      body.tool_choice = "required";
      body.parallel_tool_calls = false;
    },
    /** @param {import("../limen").ContentContext<State>} ctx */
    onContent(ctx) {
      const entries = parts(ctx.state, key);
      const event = ctx.event;
      if (event.type === "response_end") {
        if (entries.some(entry => entry.part !== "")) failed();
        delete ctx.state[key]; return;
      }
      if (event.type === "choice_end") {
        const index = entries.findIndex(entry => entry.choice === event.choice && entry.part === "");
        if (index !== -1) {
          const choice = /** @type {JsonObject} */ (entries[index]);
          if (choice.synthetic === true && choice.actual !== true) {
            if (event.finishReason === "tool_calls") event.finishReason = "stop";
            else if (!["length", "content_filter"].includes(event.finishReason)) failed();
          }
          entries.splice(index, 1);
        }
        return;
      }
      if (event.type === "part_start") {
        if (event.kind !== "tool") return;
        if (entries.filter(entry => entry.part !== "").length >= maxParts || find(entries, event) !== -1) failed();
        entries.push({ choice: event.choice, part: event.part, mode: "pending", toolName: "", pending: [], retained: 0, decoder: decoder() });
      }
      const index = find(entries, event);
      if (index === -1) return;
      const entry = /** @type {JsonObject} */ (entries[index]);
      const queue = /** @type {ContentEvent[]} */ (/** @type {unknown} */ (entry.pending));
      if (entry.mode === "actual") {
        if (event.type === "part_end") entries.splice(index, 1);
        return;
      }
      /** @type {ContentEvent[]} */
      const output = [];
      if (entry.mode === "pending") {
        queue.push(event);
        entry.retained = /** @type {number} */ (entry.retained) + JSON.stringify(event).length;

        if (event.type === "part_start" && event.kind === "tool") entry.toolName = event.tool.name ?? "";
        else if (event.type === "part_delta") entry.toolName = /** @type {string} */ (entry.toolName) + (event.tool?.name ?? "");
        const candidate = /** @type {string} */ (entry.toolName);
        if (!toolName.startsWith(candidate) || event.type === "part_end" && candidate !== toolName) {
          entry.mode = "actual";
          entry.toolName = "";
          output.push(...queue);
          const choice = recordChoice(entries, event.choice, false, maxParts);
          const start = queue.find(item => item.type === "part_start" && item.kind === "tool");
          if (start?.type !== "part_start" || start.kind !== "tool") failed();
          if (start.tool.type !== "legacy") {
            start.tool.index = /** @type {number} */ (choice.nextActualIndex);
            choice.nextActualIndex = start.tool.index + 1;
          }
          queue.length = 0;
          entry.retained = 0;
          if (event.type === "part_end") entries.splice(index, 1);
          ctx.replaceEvents(output); return;
        }
        // An exact name can still be a prefix of a real tool's fragmented name.
        // Decide only when arguments begin or the part closes, keeping empty
        // exact-name continuations under the same pending metadata bounds.
        const argumentsStarted = queue.some(item => item.type === "part_delta" && item.text !== "");
        if (candidate !== toolName || !argumentsStarted && event.type !== "part_end") {
          if (queue.length > maxPending || /** @type {number} */ (entry.retained) > bound) failed();
          ctx.replaceEvents([]); return;
        }
        if (queue.some(item =>
          item.type === "part_start" && item.kind === "tool" && item.tool.type !== "function"
          || item.type === "part_delta" && item.tool?.type === "custom")) failed();
        entry.mode = "synthetic";
        recordChoice(entries, event.choice, true, maxParts);
        output.push({ type: "part_start", choice: event.choice, part: event.part, kind: "text" });
        for (const item of queue) {
          if (item.type === "part_delta") {
            const text = decode(/** @type {JsonObject} */ (entry.decoder), item.text);
            if (text !== "") output.push({ type: "part_delta", choice: item.choice, part: item.part, text });
          }
        }
        queue.length = 0;
        entry.retained = 0;
      } else if (event.type === "part_delta") {
        if (event.tool?.name !== undefined && event.tool.name !== "") failed();
        if (event.tool?.type === "custom") failed();
        const text = decode(/** @type {JsonObject} */ (entry.decoder), event.text);
        if (text !== "") output.push({ type: "part_delta", choice: event.choice, part: event.part, text });
      }
      if (event.type === "part_end") {
        if (/** @type {JsonObject} */ (entry.decoder).phase !== 11
            && event.finishReason !== "length" && event.finishReason !== "content_filter") failed();
        entries.splice(index, 1);
        output.push(event);
      }
      ctx.replaceEvents(output);
    },
  });
}

/** @param {JsonObject[]} entries @param {number} choice @param {boolean} synthetic @param {number} maxParts @returns {JsonObject} */
function recordChoice(entries, choice, synthetic, maxParts) {
  let entry = entries.find(value => value.choice === choice && value.part === "");
  if (entry === undefined) {
    if (entries.filter(value => value.part === "").length >= maxParts) failed();
    entry = { choice, part: "", synthetic: false, actual: false, nextActualIndex: 0 };
    entries.push(entry);
  }
  entry[synthetic ? "synthetic" : "actual"] = true;
  return entry;
}
