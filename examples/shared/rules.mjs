/** @type {import("../../types/limen").PipelineStep} */
export const limitOutput = {
  name: "limit-output",
  async beforeSend({ body }) {
    const limit = body.max_completion_tokens;
    if (limit !== undefined) {
      body.max_completion_tokens = Math.min(limit, 4096);
    }
  },
};
