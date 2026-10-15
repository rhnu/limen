/** @satisfies {import("../../types/limen").Config} */
export default {
  providers: {
    primary: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: limen.env("UPSTREAM_API_KEY") ?? "example-upstream-token" },
    },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "fictional-chat-model",
    },
  },
};
