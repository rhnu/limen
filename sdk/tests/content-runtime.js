import { mapPartText, replaceText, syntheticTextTool } from "../content/index.js";

/** @typedef {import("../limen").ContentEvent} ContentEvent */
/** @typedef {import("../limen").JsonObject} JsonObject */
/** @param {unknown} condition */
function check(condition) { if (!condition) throw new Error("synthetic-content-assertion"); }
/** @param {() => unknown} action */
function rejects(action) {
  let caught = false;
  try { action(); } catch { caught = true; }
  check(caught);
}
/** @param {string} value */
function scalarString(value) {
  for (let index = 0; index < value.length; index++) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(++index);
      check(next >= 0xdc00 && next <= 0xdfff);
    } else check(unit < 0xdc00 || unit > 0xdfff);
  }
}
/** Rust's owned JSON boundary requires scalar Unicode; JSON.stringify alone does not. @param {unknown} value */
function scalarJson(value) {
  if (typeof value === "string") scalarString(value);
  else if (Array.isArray(value)) value.forEach(scalarJson);
  else if (value !== null && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) { scalarString(key); scalarJson(child); }
  }
}
/** @param {import("../limen").PipelineStep} policy @param {ContentEvent[]} events */
function run(policy, events) {
  /** @type {JsonObject} */
  let state = {};
  /** @type {ContentEvent[]} */
  const output = [];
  for (const event of events) {
    /** @type {ContentEvent[] | null} */
    let replaced = null;
    const ctx = /** @type {import("../limen").ContentContext} */ (/** @type {unknown} */ ({
      event: JSON.parse(JSON.stringify(event)), state,
      replaceEvents(/** @type {ContentEvent[]} */ values) { check(replaced === null); replaced = values; },
    }));
    policy.onContent?.(ctx);
    scalarJson(state);
    output.push(...(replaced ?? [ctx.event]));
    // The real bridge serializes between calls: no parser instance or closure may be required.
    state = JSON.parse(JSON.stringify(state));
  }
  return { output, state };
}
/** @param {ContentEvent[]} events */
function text(events) {
  return events.filter(event => event.type === "part_delta").map(event => event.text).join("");
}
/** @param {string[]} chunks @param {string} [toolName] @returns {ContentEvent[]} */
function toolEvents(chunks, toolName = "limen_emit_text") {
  return [
    { type: "part_start", choice: 0, part: "tool:0", kind: "tool", tool: { type: "function", index: 0, id: "call_public", name: toolName } },
    ...chunks.map(chunk => /** @type {ContentEvent} */ ({ type: "part_delta", choice: 0, part: "tool:0", text: chunk })),
    { type: "part_end", choice: 0, part: "tool:0" },
    { type: "choice_end", choice: 0, finishReason: "tool_calls" }, { type: "response_end" },
  ];
}

/** Deterministic ECMAScript-only suite, also callable from a real QuickJS hook. */
export function runContentPolicyTests() {
  const complete = mapPartText({
    name: "complete", kinds: ["text", "reasoning", "refusal", "tool"], maxBufferedChars: 64,
    transform: (value, info) => info.kind === "tool"
      ? JSON.stringify({ number: JSON.parse(value).number + 1 }) : value.replace(/bc/g, "X"),
  });
  for (const kind of /** @type {const} */ (["text", "reasoning", "refusal"])) {
    const mapped = run(complete, [
      { type: "part_start", choice: 0, part: kind, kind },
      { type: "part_delta", choice: 0, part: kind, text: "ab" },
      { type: "part_delta", choice: 0, part: kind, text: "cd" },
      { type: "part_end", choice: 0, part: kind }, { type: "response_end" },
    ]);
    check(text(mapped.output) === "aXd" && JSON.stringify(mapped.state) === "{}");
  }
  const mappedTool = run(complete, [
    { type: "part_start", choice: 0, part: "tool", kind: "tool", tool: { index: 0, type: "function", name: "number" } },
    { type: "part_delta", choice: 0, part: "tool", text: '{"number":', tool: { id: "call", name: "_increment" } },
    { type: "part_delta", choice: 0, part: "tool", text: "1}" },
    { type: "part_end", choice: 0, part: "tool" }, { type: "response_end" },
  ]);
  check(text(mappedTool.output) === '{"number":2}');
  const metadata = mappedTool.output[1];
  check(metadata?.type === "part_delta" && metadata.text === "" && metadata.tool?.id === "call" && metadata.tool.name === "_increment");
  let calls = 0;
  const limited = mapPartText({ name: "limited", kinds: ["text"], maxBufferedChars: 3,
    transform(value, info) { calls++; check(Object.isFrozen(info)); return value; } });
  /** @type {ContentEvent[]} */
  const open = [{ type: "part_start", choice: 0, part: "text", kind: "text" }, { type: "part_delta", choice: 0, part: "text", text: "abc" }];
  run(limited, open);
  check(calls === 0);
  rejects(() => run(limited, [...open, { type: "response_end" }]));
  rejects(() => run(limited, [...open, { type: "part_delta", choice: 0, part: "text", text: "d" }]));
  check(calls === 0);
  const cut = mapPartText({ name: "cut", kinds: ["text"], maxBufferedChars: 3,
    transform(value, info) { check(info.finishReason === "length"); return value; } });
  check(text(run(cut, [...open, { type: "part_end", choice: 0, part: "text", finishReason: "length" }, { type: "response_end" }]).output) === "abc");
  const unselected = mapPartText({ name: "selected", kinds: ["tool"], maxBufferedChars: 1, transform() { throw new Error("unexpected-mapper-call"); } });
  check(text(run(unselected, [...open, { type: "part_end", choice: 0, part: "text" }, { type: "response_end" }]).output) === "abc");
  for (const value of [null, 42, {}, Promise.resolve("text")]) {
    const bad = mapPartText({ name: "bad-return", kinds: ["text"], maxBufferedChars: 3,
      transform: /** @type {import("../content/index").MapPartTextOptions["transform"]} */ (/** @type {unknown} */ (() => value)) });
    rejects(() => run(bad, [...open, { type: "part_end", choice: 0, part: "text" }]));
  }
  rejects(() => mapPartText({ name: "duplicate", kinds: ["text", "text"], maxBufferedChars: 1, transform: value => value }));
  const onePart = mapPartText({ name: "single", kinds: ["text"], maxBufferedChars: 3, maxParts: 1, transform: value => value });
  rejects(() => run(onePart, [...open, { type: "part_start", choice: 1, part: "second", kind: "text" }]));
  /** @type {import("../content/index").PartTextKind[]} */
  const selected = ["text"];
  const copied = mapPartText({ name: "copied", kinds: selected, maxBufferedChars: 3, transform: value => value.toUpperCase() });
  selected[0] = "tool";
  check(text(run(copied, [...open, { type: "part_end", choice: 0, part: "text" }]).output) === "ABC");
  const literal = replaceText({ name: "replace", search: "aba", replacement: "X", maxBufferedChars: 2 });
  const source = "aabababa ending ab";
  for (let split = 0; split <= source.length; split++) {
    const result = run(literal, [
      { type: "part_start", choice: 0, part: "text:0", kind: "text" },
      { type: "part_delta", choice: 0, part: "text:0", text: source.slice(0, split) },
      { type: "part_delta", choice: 0, part: "text:0", text: source.slice(split) },
      { type: "part_end", choice: 0, part: "text:0" }, { type: "choice_end", choice: 0, finishReason: "stop" }, { type: "response_end" },
    ]);
    check(text(result.output) === "aXbX ending ab");
    check(JSON.stringify(result.state) === "{}");
  }
  const codec = syntheticTextTool({ name: "tool", maxBufferedChars: 512 });
  const args = ' \n{ "text" : "Hello \\n\\\"\\\\\\/\\b\\f\\r\\t\\uD83D\\uDE00 world" } \t';
  const expected = 'Hello \n"\\/\b\f\r\t😀 world';
  for (let split = 0; split <= args.length; split++) {
    const result = run(codec, toolEvents([args.slice(0, split), args.slice(split)]));
    check(text(result.output) === expected);
    check(result.output.some(event => event.type === "choice_end" && event.finishReason === "stop"));
    check(JSON.stringify(result.state) === "{}");
  }
  check(text(run(codec, toolEvents(args.split(""))).output) === expected);
  const raw = '{"text":"raw 😀"}';
  check(text(run(codec, toolEvents(raw.split(""))).output) === "raw 😀");
  check(text(run(codec, toolEvents(['{"text":"' + "x".repeat(65536) + '"}'])).output).length === 65536);
  const actual = toolEvents(['{"city":"Example"}'], "lookup_weather");
  check(JSON.stringify(run(codec, actual).output) === JSON.stringify(actual));
  const custom = toolEvents(["custom input"], "lookup_custom");
  const customStart = custom[0];
  if (customStart?.type !== "part_start" || customStart.kind !== "tool") throw new Error("synthetic-content-assertion");
  delete customStart.tool.name;
  custom.splice(1, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { type: "custom", name: "lookup_custom" } });
  check(JSON.stringify(run(codec, custom).output) === JSON.stringify(custom));
  const wrongType = toolEvents(['{"text":"answer"}']);
  wrongType.splice(1, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { type: "custom" } });
  rejects(() => run(codec, wrongType));
  for (const realFirst of [true, false]) {
    const synthetic = toolEvents(['{"text":"answer"}']).slice(0, -2);
    const real = actual.slice(0, -2).map(event => {
      if (event.type === "part_start" && event.kind === "tool") {
        return { ...event, part: "tool:1", tool: { ...event.tool, index: 1, id: "call_real" } };
      }
      return event.type === "part_delta" || event.type === "part_end" ? { ...event, part: "tool:1" } : event;
    });
    const secondReal = real.map(event => {
      if (event.type === "part_start" && event.kind === "tool") {
        return { ...event, part: "tool:3", tool: { ...event.tool, index: 3, id: "call_real_second" } };
      }
      return event.type === "part_delta" || event.type === "part_end" ? { ...event, part: "tool:3" } : event;
    });
    const mixed = run(codec, [
      ...(realFirst ? [...real, ...synthetic] : [...synthetic, ...real]), ...secondReal,
      { type: "choice_end", choice: 0, finishReason: "tool_calls" }, { type: "response_end" },
    ]);
    check(mixed.output.some(event => event.type === "choice_end" && event.finishReason === "tool_calls"));
    check(mixed.output.some(event => event.type === "part_start" && event.kind === "tool" && event.tool.name === "lookup_weather"));
    check(mixed.output.some(event => event.type === "part_start" && event.kind === "tool" && event.tool.index === 0 && event.part === "tool:1"));
    check(mixed.output.some(event => event.type === "part_start" && event.kind === "tool" && event.tool.index === 1 && event.part === "tool:3"));
    check(mixed.output.filter(event => event.type === "part_delta").map(event => event.text).join("").includes("answer"));
  }
  for (const reason of ["length", "content_filter"]) {
    const cut = toolEvents(['{"text":"visible\\uD83D']);
    for (const event of cut) {
      if (event.type === "part_end" || event.type === "choice_end") event.finishReason = reason;
    }
    const partial = run(codec, cut);
    check(text(partial.output) === "visible");
    check(partial.output.some(event => event.type === "choice_end" && event.finishReason === reason));
  }
  const fragmented = toolEvents(['{"text":"answer"}']);
  const first = fragmented[0];
  if (first?.type !== "part_start" || first.kind !== "tool") throw new Error("synthetic-content-assertion");
  first.tool.name = "limen_";
  fragmented.splice(1, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { name: "emit_text" } });
  check(text(run(codec, fragmented).output) === "answer");
  for (const exactOnStart of [true, false]) {
    const prefix = toolEvents(['{"city":"Example"}']);
    const start = prefix[0];
    if (start?.type !== "part_start" || start.kind !== "tool") throw new Error("synthetic-content-assertion");
    if (!exactOnStart) {
      delete start.tool.name;
      prefix.splice(1, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { name: "limen_emit_text" } });
    }
    prefix.splice(exactOnStart ? 1 : 2, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { name: "_extra" } });
    check(JSON.stringify(run(codec, prefix).output) === JSON.stringify(prefix));
  }
  for (const reason of ["length", "content_filter"]) {
    const empty = toolEvents([]);
    for (const event of empty) {
      if (event.type === "part_end" || event.type === "choice_end") event.finishReason = reason;
    }
    const partial = run(codec, empty);
    check(text(partial.output) === "");
    check(partial.output.some(event => event.type === "choice_end" && event.finishReason === reason));
  }
  rejects(() => run(codec, toolEvents([])));
  for (const bad of ['{"text":"unterminated', '{"text":"a","other":1}', '{"other":"x"}', '{"text":"\\q"}', '{"text":"\\uD800"}', '{"text":"\\uDC00"}', '{"text":true}', '{"text":"x"}tail']) {
    rejects(() => run(codec, toolEvents(bad.split(""))));
  }
  rejects(() => replaceText({ name: "bad", search: "long", replacement: "", maxBufferedChars: 1 }));
  rejects(() => syntheticTextTool({ name: "bad", maxBufferedChars: 1 }));
  const bounded = syntheticTextTool({ name: "bound", maxBufferedChars: 512, maxPendingEvents: 2 });
  const pending = toolEvents(["", "", ""], "limen_");
  rejects(() => run(bounded, pending));
  rejects(() => run(bounded, toolEvents(["", "", ""])));
  const pendingBytes = toolEvents([""]);
  pendingBytes.splice(1, 0, { type: "part_delta", choice: 0, part: "tool:0", text: "", tool: { id: "i".repeat(512) } });
  rejects(() => run(codec, pendingBytes));
  // Choice and part identity isolate simultaneous replacement buffers.
  const result = run(literal, [
    { type: "part_start", choice: 0, part: "same", kind: "text" },
    { type: "part_start", choice: 1, part: "same", kind: "text" },
    { type: "part_delta", choice: 0, part: "same", text: "ab" },
    { type: "part_delta", choice: 1, part: "same", text: "a" },
    { type: "part_delta", choice: 0, part: "same", text: "a" },
    { type: "part_end", choice: 0, part: "same" },
    { type: "part_end", choice: 1, part: "same" }, { type: "response_end" },
  ]);
  check(text(result.output) === "Xa");
  const originalMessages = [{ role: "user", content: "Public request" }];
  /** @type {import("../limen").RequestBody} */
  const request = { model: "example", messages: originalMessages.map(value => ({ ...value })), tools: [{ type: "function", function: { name: "lookup_weather" } }] };
  let rejected = false;
  /** @param {import("../limen").RequestBody} body */
  function prepare(body) {
    codec.beforeSend?.(/** @type {import("../limen").BeforeSendContext} */ (/** @type {unknown} */ ({ body, reject() { rejected = true; } })));
  }
  prepare(request);
  check(!rejected && request.tool_choice === "required" && request.parallel_tool_calls === false);
  check(JSON.stringify(request.messages.slice(0, -1)) === JSON.stringify(originalMessages));
  check(Array.isArray(request.tools) && request.tools.length === 2);
  for (const field of [
    { tool_choice: "required" }, { tool_choice: "none" }, { tool_choice: { type: "function", function: { name: "lookup_weather" } } },
    { functions: [] }, { function_call: "auto" }, { response_format: { type: "json_object" } }, { n: 2 }, { parallel_tool_calls: true },
    { tools: [{ type: "function", function: { name: "limen_emit_text" } }] }, { tools: [{ type: "custom" }] },
  ]) {
    rejected = false;
    const body = /** @type {import("../limen").RequestBody} */ ({ model: "example", messages: [], ...field });
    prepare(body);
    check(rejected && body.messages.length === 0);
  }
  return true;
}
