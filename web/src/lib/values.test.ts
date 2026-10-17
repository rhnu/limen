import { describe, expect, it } from "vitest";
import { dateQuery, numberText, safeChartValue, sumTokens, timeText, windowError } from "./values";

describe("known token accounting", () => {
    it("preserves unknown totals and independently counts samples", () => {
        expect(
            sumTokens([
                { sum: null, samples: 0 },
                { sum: null, samples: 0 },
            ]),
        ).toEqual({ sum: null, samples: 0 });
        expect(
            sumTokens([
                { sum: null, samples: 0 },
                { sum: "0", samples: 1 },
            ]),
        ).toEqual({ sum: "0", samples: 1 });
    });
    it("adds decimal counts beyond safe JavaScript integers exactly", () => {
        expect(
            sumTokens([
                { sum: "9007199254740993", samples: 2 },
                { sum: "10", samples: 1 },
            ]),
        ).toEqual({ sum: "9007199254741003", samples: 3 });
        expect(numberText("9007199254740993")).toBe("9,007,199,254,740,993");
    });
    it("keeps unknown and unrepresentable chart values distinct from zero", () => {
        expect(safeChartValue(null)).toBeNull();
        expect(safeChartValue("0")).toBe(0);
        expect(safeChartValue("9007199254740993")).toBeNull();
    });
});
describe("bounded query windows", () => {
    it("displays server timestamps in explicitly marked UTC", () => {
        expect(timeText("2026-01-01T08:00:00+08:00")).toBe("2026/1/1 00:00:00 UTC");
        expect(timeText(null)).toBe("未知");
    });
    it("accepts the server default and exactly 31 days", () => {
        expect(windowError("", "")).toBeNull();
        expect(windowError("2026-01-01T00:00:00Z", "2026-02-01T00:00:00Z")).toBeNull();
    });
    it("rejects partial, invalid, reversed and overly broad windows", () => {
        expect(windowError("2026-01-01T00:00", "")).not.toBeNull();
        expect(windowError("invalid", "2026-01-01")).not.toBeNull();
        expect(windowError("2026-01-02", "2026-01-01")).not.toBeNull();
        expect(windowError("2026-01-01", "2026-02-02")).not.toBeNull();
    });
    it("normalizes explicit timezone inputs to UTC without changing instants", () => {
        expect(dateQuery("2026-01-01T08:00:00+08:00", "2026-01-02T08:00:00+08:00")).toEqual({
            from: "2026-01-01T00:00:00.000Z",
            to: "2026-01-02T00:00:00.000Z",
        });
    });
});
