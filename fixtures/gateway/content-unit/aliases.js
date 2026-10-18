export default {
  providers: {
    p: {
      endpoint: "https://synthetic.invalid/chat",
      pipeline: [
        {
          name: "same",
          builtin: { kind: "replaceText", search: "ab", replacement: "bc", maxBufferedChars: 1 },
        },
      ],
    },
  },
  models: {
    chat: {
      provider: "p",
      upstream: "first",
      pipeline: [
        {
          name: "same",
          builtin: { kind: "replaceText", search: "bc", replacement: "ZZ", maxBufferedChars: 1 },
        },
      ],
    },
    alias: {
      provider: "p",
      upstream: "second",
      pipeline: [
        {
          name: "same",
          builtin: { kind: "replaceText", search: "bc", replacement: "ZZ", maxBufferedChars: 1 },
        },
      ],
    },
  },
};
