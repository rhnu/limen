import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { restoreSession } from "./restore";
import { replaceSession, useSession } from "./store";

function pendingValidation() {
    let finish: (() => void) | undefined;
    let begin: (() => void) | undefined;
    const started = new Promise<void>((resolve) => {
        begin = resolve;
    });
    const response = new Promise<Response>((resolve) => {
        finish = () =>
            resolve(
                new Response(JSON.stringify({ data: {} }), {
                    headers: { "Content-Type": "application/json" },
                }),
            );
    });
    return {
        response,
        started,
        start: () => {
            if (!begin) throw new Error("Validation not initialized");
            begin();
        },
        finish: () => {
            if (!finish) throw new Error("Validation not started");
            finish();
        },
    };
}

beforeEach(() => {
    vi.stubGlobal("window", {
        localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
        location: { origin: "http://localhost" },
    });
    replaceSession("restored-key", "verifying");
});
afterEach(() => {
    replaceSession(null, "public");
    vi.unstubAllGlobals();
});

describe("session restoration", () => {
    it("shares verification for the same session", async () => {
        const validation = pendingValidation();
        const fetch = vi.fn<typeof globalThis.fetch>(() => {
            validation.start();
            return validation.response;
        });
        vi.stubGlobal("fetch", fetch);
        const first = restoreSession();
        expect(restoreSession()).toBe(first);
        await validation.started;
        validation.finish();
        await first;
        expect(fetch).toHaveBeenCalledOnce();
        expect(useSession.getState().phase).toBe("authenticated");
    });
    it("clears invalid credentials but keeps transient failures available for manual recovery", async () => {
        const fetch = vi
            .fn<typeof globalThis.fetch>()
            .mockResolvedValueOnce(new Response(null, { status: 401 }))
            .mockResolvedValueOnce(new Response(null, { status: 503 }));
        vi.stubGlobal("fetch", fetch);
        await restoreSession();
        expect(useSession.getState()).toMatchObject({ key: null, phase: "public" });
        replaceSession("retry-key", "verifying");
        await restoreSession();
        expect(useSession.getState()).toMatchObject({ key: "retry-key", phase: "blocked" });
        expect(useSession.getState().verificationError).toMatchObject({ status: 503 });
        expect(fetch).toHaveBeenCalledTimes(2);
    });
    it("starts a new verification without waiting for obsolete work and ignores its late result", async () => {
        const old = pendingValidation();
        const current = pendingValidation();
        vi.stubGlobal(
            "fetch",
            vi
                .fn<typeof globalThis.fetch>()
                .mockImplementationOnce(() => {
                    old.start();
                    return old.response;
                })
                .mockImplementationOnce(() => {
                    current.start();
                    return current.response;
                }),
        );
        const oldRestore = restoreSession();
        await old.started;
        replaceSession("new-key", "verifying");
        const newRestore = restoreSession();
        await current.started;
        old.finish();
        await oldRestore;
        expect(useSession.getState().phase).toBe("verifying");
        expect(restoreSession()).toBe(newRestore);
        current.finish();
        await newRestore;
        expect(useSession.getState()).toMatchObject({
            key: "new-key",
            phase: "authenticated",
            verificationError: null,
        });
    });
});
