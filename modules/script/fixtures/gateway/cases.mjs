import { defineGateway } from "limen:gateway";

const options = { endpoint: "https://fictional.invalid/chat" };
const empty = () => {};
export const cases = [
  {},
  () => Promise.resolve({}),
  () => 1,
  new Proxy(() => ({}), {}),
  defineGateway(() => ({})),
  defineGateway(async () => {}),
  defineGateway((gateway) => {
    gateway.provider("p", options);
    gateway.provider("p", options);
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).model("m", { upstream: "one" });
    gateway.provider("q", options).model("m", { upstream: "two" });
  }),
  defineGateway((gateway) => {
    const provider = gateway.provider("p", options);
    provider.use({ name: "same", beforeSend: empty });
    provider.step("same", (step) => {
      step.onResponse(empty);
    });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).step("empty", empty);
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).step("async", async (step) => {
      step.beforeSend(empty);
    });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).step("returned", (step) => {
      step.beforeSend(empty);
      return {};
    });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", { ...options, pipeline: [] });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).model("m", { provider: "q", upstream: "one" });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", {
      get endpoint() {
        throw Error("accessor executed");
      },
    });
  }),
  defineGateway((gateway) => {
    gateway.provider("p", options).use(new Proxy({ name: "proxy", beforeSend: empty }, {}));
  }),
  ...["onRequest", "onErrorResponse", "onError", "onFinish"].map((stage) =>
    defineGateway((gateway) => {
      gateway[stage](empty);
      gateway[stage](empty);
    }),
  ),
  ...[
    "beforeSend",
    "afterReceive",
    "onResponse",
    "onContent",
    "onUpstreamEvent",
    "onStreamEvent",
    "onErrorResponse",
  ].map((stage) =>
    defineGateway((gateway) => {
      gateway.provider("p", options).step("duplicate", (step) => {
        step[stage](empty);
        step[stage](empty);
      });
    }),
  ),
  defineGateway(() => {
    throw new RangeError("private setup failure");
  }),
];
