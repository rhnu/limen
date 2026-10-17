import { capCompletionTokens, dropRequestFields, forwardClientHeader } from "../../../sdk/policies/index.js";
import { endpoint } from "../endpoint.js";

const capOptions = { name: "unit-cap", max: 16, onChange: "capped" };
const sharedCap = capCompletionTokens(capOptions);
capOptions.max = 1;
capOptions.onChange = "must_not_be_read";
const fields = ["store", "metadata"];
const dropOptions = { name: "unit-drop", fields, onChange: "scrubbed" };
const sharedDrop = dropRequestFields(dropOptions);
fields[0] = "vendor";
fields.push("messages");
dropOptions.onChange = "must_not_be_read";
const headerOptions = { name: "unit-header", from: "X-Profile", to: "X-Upstream-Profile" };
const sharedForward = forwardClientHeader(headerOptions);
headerOptions.from = "must-not-be-read";
headerOptions.to = "must-not-be-read";
const sharedLegacy = capCompletionTokens({ name: "unit-legacy", max: 16, field: "max_tokens", onChange: "capped" });

function require(value) {
  if (!value) throw new Error("synthetic-policy-assertion");
}

function checkParameters() {
  let checked = 0;
  function invalid(create) {
    let rejected = false;
    try { create(); } catch (error) {
      require(error instanceof TypeError && error.message === "invalid policy configuration");
      rejected = true;
    }
    require(rejected);
    checked++;
  }
  for (const value of [null, [], 1, "synthetic-secret", undefined]) {
    invalid(() => capCompletionTokens(value));
    invalid(() => dropRequestFields(value));
    invalid(() => forwardClientHeader(value));
  }
  for (const name of ["", "bad:name", "with space", "bad\n", "中", "a".repeat(129), null]) {
    invalid(() => capCompletionTokens({ name, max: 1 }));
    invalid(() => dropRequestFields({ name, fields: ["store"] }));
    invalid(() => forwardClientHeader({ name, from: "x-profile", to: "x-upstream" }));
  }
  for (const max of [0, -1, 0.5, 9007199254740992, Infinity, NaN, null, "1", true]) {
    invalid(() => capCompletionTokens({ name: "cap", max }));
  }
  for (const field of ["output_tokens", "", null, 1]) {
    invalid(() => capCompletionTokens({ name: "cap", max: 1, field }));
  }
  for (const onChange of ["", "bad-name", "Bad", "code\n", "a".repeat(65), null, 1]) {
    invalid(() => capCompletionTokens({ name: "cap", max: 1, onChange }));
    invalid(() => dropRequestFields({ name: "drop", fields: ["store"], onChange }));
  }
  for (const fields of [[], null, "store", [""], [1], ["store", "store"], ["model"], ["messages"], ["stream"]]) {
    invalid(() => dropRequestFields({ name: "drop", fields }));
  }
  for (const header of ["", "bad name", "bad:name", "x\n", "中", null, 1,
    "Authorization", "Proxy-Authorization", "Cookie", "Set-Cookie", "Proxy-Authenticate",
    "Host", "Content-Length", "Transfer-Encoding", "Connection", "Content-Encoding",
    "Content-Type", "Proxy-Connection", "Keep-Alive", "TE", "Trailer", "Upgrade"]) {
    invalid(() => forwardClientHeader({ name: "header", from: header, to: "x-upstream" }));
    invalid(() => forwardClientHeader({ name: "header", from: "x-profile", to: header }));
  }
  require(Object.isFrozen(capCompletionTokens({ name: "a".repeat(128), max: Number.MAX_SAFE_INTEGER })));
  require(Object.isFrozen(dropRequestFields({ name: "0-._/", fields: ["store"] })));
  require(Object.isFrozen(forwardClientHeader({ name: "header", from: "X-Profile", to: "x_token" })));
  // These JS numbers cannot cross the gateway's owned-JSON bridge as request input.
  for (const value of [Number.MAX_SAFE_INTEGER + 1, Infinity, NaN, undefined]) {
    let rejected;
    sharedCap.beforeSend({
      body: { max_completion_tokens: value },
      reject(status) { rejected = status; },
      observe: { record() { throw new Error("synthetic-unexpected-observation"); } },
    });
    require(rejected === 422);
  }
  return checked;
}

export default {
  onRequest(ctx) {
    if (ctx.body.unit === "parameters") ctx.state.checked = checkParameters();
    else if (ctx.body.unit === "cap") sharedCap.beforeSend(ctx);
    else if (ctx.body.unit === "legacy") sharedLegacy.beforeSend(ctx);
    else if (ctx.body.unit === "drop") sharedDrop.beforeSend(ctx);
    else if (ctx.body.unit === "header") sharedForward.beforeSend(ctx);
  },
  providers: {
    p: {
      endpoint,
      headers: { "X-Upstream-Profile": "static" },
      pipeline: [
        capCompletionTokens({ name: "provider-cap", max: 64, onChange: "capped" }),
        dropRequestFields({ name: "privacy", fields: ["store", "metadata"], onChange: "scrubbed" }),
        forwardClientHeader({ name: "profile", from: "X-Profile", to: "X-Upstream-Profile" }),
      ],
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "real-model",
      pipeline: [capCompletionTokens({ name: "model-cap", max: 16, onChange: "capped" })],
    },
  },
};
