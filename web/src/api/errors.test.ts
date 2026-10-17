import { describe, expect, it } from "vitest";
import { mapFailure } from "./errors";

describe("controlled API errors", () => {
    it("uses approved text and bounded request identity rather than arbitrary messages", () => {
        const failure = mapFailure(
            { error: { code: "busy", message: "secret raw exception", request_id: "run.12" } },
            new Response(null, { status: 503 }),
        );
        expect(failure.code).toBe("busy");
        expect(failure.status).toBe(503);
        expect(failure.requestId).toBe("run.12");
        expect(failure.message).not.toContain("secret");
    });
    it("treats every HTTP 401 as authentication loss even without JSON", () => {
        expect(mapFailure("private body", new Response(null, { status: 401 })).code).toBe(
            "unauthorized",
        );
    });
    it.each([
        null,
        false,
        [],
        { error: null },
        { error: "private payload" },
        { error: { code: 123, request_id: {} } },
    ])("handles malformed error envelopes without leaking their contents", (error) => {
        const failure = mapFailure(error, new Response(null, { status: 500 }));
        expect(failure.code).toBe("invalid_response");
        expect(failure.requestId).toBeUndefined();
    });
    it("rejects unapproved codes and arbitrary identities", () => {
        const failure = mapFailure(
            { error: { code: "untrusted payload", request_id: "<secret>" } },
            new Response(null, { status: 500 }),
        );
        expect(failure.code).toBe("invalid_response");
        expect(failure.requestId).toBeUndefined();
        expect(mapFailure(new TypeError("sensitive network detail")).code).toBe("network_error");
        expect(
            mapFailure({ error: { code: "__proto__" } }, new Response(null, { status: 500 })).code,
        ).toBe("invalid_response");
    });
});
