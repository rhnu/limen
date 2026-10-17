(/** @param {import("../internal/types").Intrinsics} i @returns {import("../internal/types").PhaseFactory} */
function (i) {
  "use strict";
  return function phasePolicy(phase) {
    if (phase !== "onRequest" && phase !== "beforeSend" && phase !== "afterReceive"
        && phase !== "onResponse" && phase !== "onContent" && phase !== "onUpstreamEvent" && phase !== "onStreamEvent"
        && phase !== "onErrorResponse" && phase !== "onError" && phase !== "onFinish") i.fail();
    const eventHook = phase === "onContent" || phase === "onUpstreamEvent" || phase === "onStreamEvent";
    return {
      eventHook,
      observer: phase === "onError" || phase === "onFinish",
      headers: phase === "beforeSend" || phase === "afterReceive" || phase === "onErrorResponse",
      reject: phase === "onRequest" || phase === "beforeSend",
      writable(key) {
        return (key === "body" && (phase === "onRequest" || phase === "beforeSend" || phase === "onResponse" || phase === "onErrorResponse"))
          || (key === "event" && eventHook) || (key === "url" && phase === "beforeSend")
          || (key === "status" && (phase === "afterReceive" || phase === "onErrorResponse"))
          || (key === "contentType" && phase === "onErrorResponse");
      },
    };
  };
})
