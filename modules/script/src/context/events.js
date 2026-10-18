"use strict";
{
  /**
   * @param {import("../internal/types").Intrinsics} i @param {number} maxEvents
   * @returns {import("../internal/types").EventFactory}
   */
  function createEvents(i, maxEvents) {
    const { descriptor, fail: invalid, isArray, read, freeze } = i;
    /** @returns {never} */
    function fail() {
      return invalid();
    }
    return function eventDraft(root) {
      if (
        !descriptor(root, "event") ||
        descriptor(root, "events") ||
        descriptor(root, "replaceEvents")
      )
        fail();
      /** @type {unknown[] | undefined} */
      let replacement;
      return {
        replace: freeze(function (events) {
          if (replacement !== undefined || !isArray(events) || events.length > maxEvents) fail();
          replacement = events;
        }),
        snapshot() {
          const events = replacement === undefined ? [read(root, "event")] : replacement;
          // A pending hook can retain and grow the supplied array before settlement.
          if (events.length > maxEvents) fail();
          for (let index = 0; index < events.length; index++) {
            const event = read(events, index);
            if (event === null || typeof event !== "object" || isArray(event)) fail();
          }
          return events;
        },
      };
    };
  }
  createEvents;
}
