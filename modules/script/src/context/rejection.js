"use strict";
{
  /** @param {import("../internal/types").Intrinsics} i @returns {import("../internal/types").RejectionFactory} */
  function createRejection(i) {
    const { freeze, fail: invalid } = i;
    /** @returns {never} */
    function fail() {
      return invalid();
    }
    return function rejectionDraft() {
      /** @type {{status: number} | undefined} */
      let rejection;
      return {
        reject: freeze(function (status = 400) {
          if (
            arguments.length > 1 ||
            rejection ||
            (status !== 400 && status !== 403 && status !== 422)
          )
            fail();
          rejection = freeze({ status });
        }),
        snapshot() {
          return rejection;
        },
      };
    };
  }
  createRejection;
}
