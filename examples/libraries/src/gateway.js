import { integer, maxValue, minValue, number, object, optional, pipe, safeParse } from "valibot";
import { defineGateway } from "limen:gateway";
import { capCompletionTokens, dropRequestFields } from "limen:policies";

// These validators use ordinary ECMAScript only; no URL, DOM or Node.js APIs.
const requestOptions = object({
  temperature: optional(pipe(number(), minValue(0), maxValue(2))),
  max_completion_tokens: optional(pipe(number(), integer(), minValue(1))),
});

/** @param {import("limen:gateway").ProviderScope} scope */
function useProviderPolicy(scope) {
  scope.step("request-policy", (step) => {
    step.beforeSend((ctx) => {
      // This example provider does not support logprobs, even when false.
      if ("logprobs" in ctx.body) {
        ctx.reject(400);
        return;
      }
      // Reading client headers does not forward them; this value is explicit.
      ctx.headers.set("X-Policy", "validated");
    });
  });
  scope.use(capCompletionTokens({ name: "cap-output", max: 4096 }));
  scope.use(dropRequestFields({ name: "remove-store", fields: ["store"] }));
}

export default defineGateway((gateway) => {
  gateway.onRequest((ctx) => {
    const policy = ctx.client.headers.getAll("x-example-policy");
    if (policy.length !== 1 || policy[0] !== "allow") {
      ctx.reject(403);
      return;
    }
    if (!safeParse(requestOptions, ctx.body).success) {
      ctx.reject(422);
      return;
    }
    // Validation is a predicate. Keeping the draft preserves unknown extensions.
    ctx.state.policy = "allowed";
  });
  const demo = gateway.provider("demo", {
    endpoint: "https://example.invalid/v1/chat/completions",
  });
  useProviderPolicy(demo);
  demo.model("chat", { upstream: "demo-model" });
});
