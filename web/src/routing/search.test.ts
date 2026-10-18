import { describe, expect, it } from "vitest";
import {
    configSearchSchema,
    emptySearchSchema,
    logsSearchSchema,
    parseRequestId,
    parseSearch,
    requestsSearchSchema,
    serializeSearch,
    usageSearchSchema,
} from "./search";

describe("management search parameters", () => {
    it("normalizes omitted and empty filters to the same defaults", () => {
        expect(parseSearch(usageSearchSchema, new URLSearchParams()).filters).toEqual({
            from: "",
            to: "",
            groupBy: "day",
        });
        const empty = parseSearch(
            usageSearchSchema,
            new URLSearchParams("from=&to=&model=%20&provider=&groupBy="),
        );
        expect(empty.error).toBeNull();
        expect(empty.canonical).toBe("groupBy=day");
        expect(parseSearch(configSearchSchema, new URLSearchParams("section=")).filters).toEqual({
            section: "server",
        });
        expect(
            parseSearch(requestsSearchSchema, new URLSearchParams("state=&outcome=")).canonical,
        ).toBe("");
        expect(
            parseSearch(logsSearchSchema, new URLSearchParams("minLevel=&fileId=&requestId="))
                .canonical,
        ).toBe("");
    });

    it("round trips normalized local times, text and enums in stable order", () => {
        const first = parseSearch(
            requestsSearchSchema,
            new URLSearchParams({
                state: "finalized",
                model: " 模型 / + & ",
                to: "2026-01-02T08:00:00.000",
                from: "2026-01-01T08:00:00",
                outcome: "success",
            }),
        );
        expect(first.error).toBeNull();
        expect(first.filters).toMatchObject({
            from: "2026-01-01T08:00",
            to: "2026-01-02T08:00",
            model: "模型 / + &",
            state: "finalized",
            outcome: "success",
        });
        expect(first.filters).not.toBeNull();
        if (!first.filters) throw new Error("Expected valid test filters");
        const second = parseSearch(requestsSearchSchema, serializeSearch(first.filters));
        expect(second).toEqual(first);
        expect(
            parseSearch(
                requestsSearchSchema,
                new URLSearchParams([...serializeSearch(first.filters)].reverse()),
            ).canonical,
        ).toBe(first.canonical);
    });

    it("preserves nonzero seconds and millisecond precision", () => {
        const result = parseSearch(
            usageSearchSchema,
            new URLSearchParams({ from: "2026-01-01T08:00:00.010", to: "2026-01-01T08:00:01.100" }),
        );
        expect(result.filters).toMatchObject({
            from: "2026-01-01T08:00:00.01",
            to: "2026-01-01T08:00:01.1",
        });
    });

    it.each([
        "model=a&model=b",
        "model=&model=",
        "model=a&%6dodel=b",
        "cursor=private-cursor",
        "limit=100",
        "unknown=private-value",
        "__proto__=private-value",
        "constructor=private-value",
    ])("rejects duplicates and unsupported fields: %s", (value) => {
        const result = parseSearch(requestsSearchSchema, new URLSearchParams(value));
        expect(result.filters).toBeNull();
        expect(result.error).toBeTruthy();
        expect(result.error).not.toContain("private");
        expect(result.canonical).toBe("");
    });

    it("rejects invalid enums rather than silently selecting defaults", () => {
        for (const result of [
            parseSearch(usageSearchSchema, new URLSearchParams("groupBy=invalid-secret")),
            parseSearch(requestsSearchSchema, new URLSearchParams("state=invalid-secret")),
            parseSearch(requestsSearchSchema, new URLSearchParams("outcome=invalid-secret")),
            parseSearch(logsSearchSchema, new URLSearchParams("minLevel=invalid-secret")),
            parseSearch(configSearchSchema, new URLSearchParams("section=invalid-secret")),
        ]) {
            expect(result.filters).toBeNull();
            expect(result.error).not.toContain("invalid-secret");
        }
    });

    it("allows only UTF-8 text within the API filter bound", () => {
        expect(
            parseSearch(requestsSearchSchema, new URLSearchParams({ model: "a".repeat(128) }))
                .error,
        ).toBeNull();
        expect(
            parseSearch(requestsSearchSchema, new URLSearchParams({ model: "a".repeat(129) }))
                .filters,
        ).toBeNull();
        expect(
            parseSearch(requestsSearchSchema, new URLSearchParams({ provider: "模".repeat(43) }))
                .filters,
        ).toBeNull();
    });

    it("accepts exactly 31 days and rejects incomplete, reversed or broader windows", () => {
        const from = "2026-01-01T00:00";
        expect(
            parseSearch(usageSearchSchema, new URLSearchParams({ from, to: "2026-02-01T00:00" }))
                .error,
        ).toBeNull();
        for (const params of [
            { from },
            { to: from },
            { from, to: from },
            { from, to: "2025-12-31T23:59" },
            { from, to: "2026-02-01T00:00:00.001" },
        ])
            expect(parseSearch(usageSearchSchema, serializeSearch(params)).filters).toBeNull();
    });

    it.each([
        "private-invalid-time",
        "2026-02-30T08:00",
        "2026-13-01T08:00",
        "2026-01-01T24:00",
        "2026-01-01",
        "2026-01-01T08:00Z",
        "2026-01-01T08:00+08:00",
    ])("rejects invalid or nonlocal datetime values: %s", (from) => {
        const result = parseSearch(
            usageSearchSchema,
            new URLSearchParams({ from, to: "2026-03-01T08:00" }),
        );
        expect(result.filters).toBeNull();
        expect(result.error).not.toContain(from);
    });

    it("accepts leap days and refuses parameters on pages without filters", () => {
        expect(
            parseSearch(
                usageSearchSchema,
                new URLSearchParams({ from: "2028-02-29T08:00", to: "2028-03-01T08:00" }),
            ).error,
        ).toBeNull();
        expect(parseSearch(emptySearchSchema, new URLSearchParams()).error).toBeNull();
        expect(
            parseSearch(emptySearchSchema, new URLSearchParams("token=private-value")).filters,
        ).toBeNull();
        expect(
            parseSearch(configSearchSchema, new URLSearchParams("section=models&section=models"))
                .filters,
        ).toBeNull();
    });

    it("validates opaque path and log identities without exposing rejected input", () => {
        expect(parseRequestId("request-ABC-123")).toEqual({ id: "request-ABC-123", error: null });
        expect(parseRequestId("a".repeat(128)).error).toBeNull();
        expect(parseRequestId("")).toEqual({ id: null, error: "请求 ID 无效。" });
        expect(parseRequestId(undefined)).toEqual({ id: null, error: "请求 ID 无效。" });
        for (const id of ["a".repeat(129), "private/path", "private.id", "private%2Fid", "请求"]) {
            expect(parseRequestId(id)).toEqual({ id: null, error: "请求 ID 无效。" });
            expect(
                parseSearch(logsSearchSchema, new URLSearchParams({ requestId: id })).filters,
            ).toBeNull();
            expect(
                parseSearch(logsSearchSchema, new URLSearchParams({ fileId: id })).filters,
            ).toBeNull();
        }
    });
});

describe("performance and transport filters", () => {
    it("round trips false stream intent and independent performance outcome", () => {
        const params = new URLSearchParams(
            "clientStream=false&upstreamMedia=json&performanceOutcome=cancelled",
        );
        const parsed = parseSearch(usageSearchSchema, params);
        expect(parsed.error).toBeNull();
        expect(parsed.filters).toMatchObject({
            clientStream: "false",
            upstreamMedia: "json",
            performanceOutcome: "cancelled",
        });
        expect(parsed.canonical).toBe(
            "clientStream=false&groupBy=day&performanceOutcome=cancelled&upstreamMedia=json",
        );
        expect(
            parseSearch(
                requestsSearchSchema,
                new URLSearchParams("clientStream=true&upstreamMedia=sse"),
            ).filters,
        ).toMatchObject({ clientStream: "true", upstreamMedia: "sse" });
    });
    it.each([
        "clientStream=0",
        "clientStream=True",
        "upstreamMedia=application%2Fjson",
        "performanceOutcome=pending",
    ])("rejects unsupported values %s", (raw) => {
        expect(parseSearch(usageSearchSchema, new URLSearchParams(raw)).filters).toBeNull();
    });
    it("keeps performance outcome out of request-list filters", () => {
        expect(
            parseSearch(requestsSearchSchema, new URLSearchParams("performanceOutcome=all"))
                .filters,
        ).toBeNull();
    });
});
