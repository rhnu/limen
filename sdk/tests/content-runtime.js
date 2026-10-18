import { mapPartText } from "limen:content";

/** @typedef {import("../limen").ContentEvent} ContentEvent */
/** @typedef {import("../limen").JsonObject} JsonObject */
/** @param {unknown} condition */
function check(condition) {
  if (!condition) throw new Error("synthetic-content-assertion");
}
/** @param {() => unknown} action */
function rejects(action) {
  let caught = false;
  try {
    action();
  } catch {
    caught = true;
  }
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
    for (const [key, child] of Object.entries(value)) {
      scalarString(key);
      scalarJson(child);
    }
  }
}
/** @param {import("../limen").ScriptStep} policy @param {ContentEvent[]} events */
function run(policy, events) {
  /** @type {JsonObject} */
  let state = {};
  /** @type {ContentEvent[]} */
  const output = [];
  for (const event of events) {
    /** @type {ContentEvent[] | null} */
    let replaced = null;
    const ctx = /** @type {import("../limen").ContentContext} */ (
      /** @type {unknown} */ ({
        event: JSON.parse(JSON.stringify(event)),
        state,
        replaceEvents(/** @type {ContentEvent[]} */ values) {
          check(replaced === null);
          replaced = values;
        },
      })
    );
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
  return events
    .filter((event) => event.type === "part_delta")
    .map((event) => event.text)
    .join("");
}
/** Deterministic ECMAScript-only suite, also callable from a real QuickJS hook. */
export function runContentPolicyTests() {
  const complete = mapPartText({
    name: "complete",
    kinds: ["text", "reasoning", "refusal", "tool"],
    maxBufferedChars: 64,
    transform: (value, info) =>
      info.kind === "tool"
        ? JSON.stringify({ number: JSON.parse(value).number + 1 })
        : value.replace(/bc/g, "X"),
  });
  for (const kind of /** @type {const} */ (["text", "reasoning", "refusal"])) {
    const mapped = run(complete, [
      { type: "part_start", choice: 0, part: kind, kind },
      { type: "part_delta", choice: 0, part: kind, text: "ab" },
      { type: "part_delta", choice: 0, part: kind, text: "cd" },
      { type: "part_end", choice: 0, part: kind },
      { type: "response_end" },
    ]);
    check(text(mapped.output) === "aXd" && JSON.stringify(mapped.state) === "{}");
  }
  const mappedTool = run(complete, [
    {
      type: "part_start",
      choice: 0,
      part: "tool",
      kind: "tool",
      tool: { index: 0, type: "function", name: "number" },
    },
    {
      type: "part_delta",
      choice: 0,
      part: "tool",
      text: '{"number":',
      tool: { id: "call", name: "_increment" },
    },
    { type: "part_delta", choice: 0, part: "tool", text: "1}" },
    { type: "part_end", choice: 0, part: "tool" },
    { type: "response_end" },
  ]);
  check(text(mappedTool.output) === '{"number":2}');
  const metadata = mappedTool.output[1];
  check(
    metadata?.type === "part_delta" &&
      metadata.text === "" &&
      metadata.tool?.id === "call" &&
      metadata.tool.name === "_increment",
  );
  let calls = 0;
  const limited = mapPartText({
    name: "limited",
    kinds: ["text"],
    maxBufferedChars: 3,
    transform(value, info) {
      calls++;
      check(Object.isFrozen(info));
      return value;
    },
  });
  /** @type {ContentEvent[]} */
  const open = [
    { type: "part_start", choice: 0, part: "text", kind: "text" },
    { type: "part_delta", choice: 0, part: "text", text: "abc" },
  ];
  run(limited, open);
  check(calls === 0);
  rejects(() => run(limited, [...open, { type: "response_end" }]));
  rejects(() =>
    run(limited, [...open, { type: "part_delta", choice: 0, part: "text", text: "d" }]),
  );
  check(calls === 0);
  const cut = mapPartText({
    name: "cut",
    kinds: ["text"],
    maxBufferedChars: 3,
    transform(value, info) {
      check(info.finishReason === "length");
      return value;
    },
  });
  check(
    text(
      run(cut, [
        ...open,
        { type: "part_end", choice: 0, part: "text", finishReason: "length" },
        { type: "response_end" },
      ]).output,
    ) === "abc",
  );
  const unselected = mapPartText({
    name: "selected",
    kinds: ["tool"],
    maxBufferedChars: 1,
    transform() {
      throw new Error("unexpected-mapper-call");
    },
  });
  check(
    text(
      run(unselected, [
        ...open,
        { type: "part_end", choice: 0, part: "text" },
        { type: "response_end" },
      ]).output,
    ) === "abc",
  );
  for (const value of [null, 42, {}, Promise.resolve("text")]) {
    const bad = mapPartText({
      name: "bad-return",
      kinds: ["text"],
      maxBufferedChars: 3,
      transform: /** @type {import("limen:content").MapPartTextOptions["transform"]} */ (
        /** @type {unknown} */ (() => value)
      ),
    });
    rejects(() => run(bad, [...open, { type: "part_end", choice: 0, part: "text" }]));
  }
  rejects(() =>
    mapPartText({
      name: "duplicate",
      kinds: ["text", "text"],
      maxBufferedChars: 1,
      transform: (value) => value,
    }),
  );
  const onePart = mapPartText({
    name: "single",
    kinds: ["text"],
    maxBufferedChars: 3,
    maxParts: 1,
    transform: (value) => value,
  });
  rejects(() =>
    run(onePart, [...open, { type: "part_start", choice: 1, part: "second", kind: "text" }]),
  );
  /** @type {import("limen:content").PartTextKind[]} */
  const selected = ["text"];
  const copied = mapPartText({
    name: "copied",
    kinds: selected,
    maxBufferedChars: 3,
    transform: (value) => value.toUpperCase(),
  });
  selected[0] = "tool";
  check(
    text(run(copied, [...open, { type: "part_end", choice: 0, part: "text" }]).output) === "ABC",
  );
  return true;
}
