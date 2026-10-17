import { describe, expect, it } from "vitest";
import { initialCursor, moveCursor } from "./pagination";

describe("bounded cursor navigation", () => {
    it("visits empty windows when the server supplies a next cursor and goes back", () => {
        const first = initialCursor("filter-a");
        const second = moveCursor(first, "filter-a", { type: "next", cursor: "opaque" });
        expect(second.cursors.at(-1)).toBe("opaque");
        expect(moveCursor(second, "filter-a", { type: "previous" })).toEqual(first);
        expect(moveCursor(first, "filter-a", { type: "previous" })).toBe(first);
        expect(moveCursor(first, "filter-a", { type: "next", cursor: null })).toBe(first);
    });
    it("drops cursors when filters change or the query is reset", () => {
        const second = moveCursor(initialCursor("a"), "a", { type: "next", cursor: "old" });
        expect(moveCursor(second, "b", { type: "previous" }).cursors).toEqual([undefined]);
        expect(moveCursor(second, "a", { type: "reset" }).cursors).toEqual([undefined]);
    });
    it("bounds the number of visited pages", () => {
        let state = initialCursor("a");
        for (let page = 1; page <= 150; page++) {
            state = moveCursor(state, "a", { type: "next", cursor: String(page) });
        }
        expect(state.cursors).toHaveLength(100);
        expect(state.cursors.at(-1)).toBe("99");
    });
});
