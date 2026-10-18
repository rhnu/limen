"use strict";
{
  /**
   * @param {import("../internal/types").Intrinsics} i
   * @param {(name: string, value: string, mutation: boolean) => string | null | undefined} validateHeader
   * @param {number} maxNodes @param {number} maxBytes
   * @returns {import("../internal/types").HeaderFactory}
   */
  function createHeaders(i, validateHeader, maxNodes, maxBytes) {
    const { isArray, fail: invalid, read, setProto, freeze, ownKeys } = i;
    /** @returns {never} */
    function fail() {
      return invalid();
    }
    return function headerDraft(input, writable, readOnly = false) {
      if (!isArray(input)) fail();
      /** @type {import("../internal/types").HeaderField[]} */
      let fields = setProto([], null);
      let bytes = 0;
      /** @param {unknown} name @param {unknown} value @param {boolean} mutation */
      function normalized(name, value, mutation) {
        if (typeof name !== "string" || typeof value !== "string") fail();
        const normalized = validateHeader(name, value, mutation);
        if (typeof normalized !== "string") fail();
        return normalized;
      }
      /** @param {string} name @param {unknown} value */
      function appendField(name, value) {
        if (typeof value !== "string") fail();
        const added = name.length + value.length;
        if (fields.length >= maxNodes || added + bytes > maxBytes) fail();
        fields[fields.length] = freeze({ name, value });
        bytes += added;
      }
      for (let index = 0; index < input.length; index++) {
        const field = input[index];
        appendField(
          normalized(read(field, "name"), read(field, "value"), false),
          read(field, "value"),
        );
      }
      /** @param {string} name */
      function remove(name) {
        /** @type {import("../internal/types").HeaderField[]} */
        const next = setProto([], null);
        let nextBytes = 0;
        for (let index = 0; index < fields.length; index++) {
          const field = fields[index];
          if (!field) fail();
          if (field.name !== name) {
            next[next.length] = field;
            nextBytes += field.name.length + field.value.length;
          }
        }
        fields = next;
        bytes = nextBytes;
      }
      /** @param {unknown} name @param {unknown} value */
      function mutation(name, value) {
        if (!writable) fail();
        return normalized(name, value, true);
      }
      /** @type {import("../internal/types").HeaderApi} */
      const api = {
        getAll(name) {
          const normalizedName = normalized(name, "", false);
          const values = [];
          for (let index = 0; index < fields.length; index++) {
            const field = fields[index];
            if (!field) fail();
            if (field.name === normalizedName) values[values.length] = field.value;
          }
          return values;
        },
      };
      if (!readOnly) {
        api.append = (name, value) => {
          appendField(mutation(name, value), value);
        };
        api.set = (name, value) => {
          const normalizedName = mutation(name, value);
          remove(normalizedName);
          appendField(normalizedName, value);
        };
        api.delete = (name) => {
          remove(mutation(name, ""));
        };
      }
      freeze(api);
      const methods = ownKeys(api);
      for (let index = 0; index < methods.length; index++)
        freeze(read(api, /** @type {PropertyKey} */ (methods[index])));
      return {
        api,
        snapshot() {
          const output = [];
          for (let index = 0; index < fields.length; index++) {
            const field = fields[index];
            if (!field) fail();
            output[output.length] = field;
          }
          return output;
        },
      };
    };
  }
  createHeaders;
}
