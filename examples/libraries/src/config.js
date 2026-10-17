import { integer, maxValue, minValue, number, object, optional, pipe, safeParse } from "valibot";
import { capCompletionTokens, dropRequestFields } from "../../../sdk/policies/index.js";

// These validators use ordinary ECMAScript only; no URL, DOM or Node.js APIs.
const requestOptions = object({
  temperature: optional(pipe(number(), minValue(0), maxValue(2))),
  max_completion_tokens: optional(pipe(number(), integer(), minValue(1))),
});

/** @satisfies {import("../../../sdk/limen").Config} */
export default {
  onRequest(ctx) {
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
  },
  providers: {
    demo: {
      endpoint: "https://example.invalid/v1/chat/completions",
      pipeline: [
        {
          name: "request-policy",
          beforeSend(ctx) {
            // This example provider does not support logprobs, even when false.
            if ("logprobs" in ctx.body) {
              ctx.reject(400);
              return;
            }
            // Reading client headers does not forward them; this value is explicit.
            ctx.headers.set("X-Policy", "validated");
          },
        },
        capCompletionTokens({ name: "cap-output", max: 4096 }),
        dropRequestFields({ name: "remove-store", fields: ["store"] }),
      ],
    },
  },
  models: { chat: { provider: "demo", upstream: "demo-model" } },
};
