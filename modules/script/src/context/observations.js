"use strict";
{
  /**
   * @param {import("../internal/types").Intrinsics} i
   * @param {string[]} names @param {number} maxObservations
   * @returns {import("../internal/types").ObservationFactory}
   */
  function createObservations(i, names, maxObservations) {
    const { create, freeze, descriptor, setProto, define, fail: invalid } = i;
    /** @returns {never} */
    function fail() {
      return invalid();
    }
    /** @type {Record<string, number>} */
    const ids = create(null);
    for (let index = 0; index < names.length; index++) {
      const name = names[index];
      if (name === undefined) fail();
      ids[name] = index;
    }
    freeze(ids);
    return function observations() {
      /** @type {number[]} */
      const events = setProto([], null);
      let dropped = 0;
      return {
        api: freeze({
          record: freeze(function (/** @type {unknown} */ name) {
            if (arguments.length !== 1 || typeof name !== "string") fail();
            const registration = descriptor(ids, name);
            if (!registration) fail();
            const id = /** @type {number} */ (registration.value);
            for (let index = 0; index < events.length; index++) if (events[index] === id) return;
            if (events.length >= maxObservations) {
              if (dropped < 9007199254740991) dropped++;
              return;
            }
            events[events.length] = id;
          }),
        }),
        snapshot() {
          /** @type {number[]} */
          const output = [];
          for (let index = 0; index < events.length; index++) {
            define(output, index, {
              value: events[index],
              enumerable: true,
              writable: true,
              configurable: true,
            });
          }
          return { events: output, dropped };
        },
      };
    };
  }
  createObservations;
}
