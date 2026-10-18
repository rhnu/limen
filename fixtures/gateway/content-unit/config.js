export default {
  providers: {
    p: {
      endpoint: "https://synthetic.invalid/chat",
      pipeline: [
        {
          name: "replace",
          builtin: { kind: "replaceText", search: "ab", replacement: "ZZ", maxBufferedChars: 1 },
        },
      ],
    },
  },
  models: { chat: { provider: "p", upstream: "native" } },
};
