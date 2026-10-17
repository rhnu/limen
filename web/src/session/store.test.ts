import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    isCurrentSession,
    onSessionReset,
    readKey,
    replaceSession,
    storageKey,
    useSession,
    writeKey,
} from "./store";
import { api, validateKey } from "../api/client";

function memoryStorage() {
    const data = new Map<string, string>();
    return {
        getItem: (key: string) => data.get(key) ?? null,
        setItem: (key: string, value: string) => {
            data.set(key, value);
        },
        removeItem: (key: string) => {
            data.delete(key);
        },
    };
}
beforeEach(() => {
    vi.stubGlobal("window", {
        localStorage: memoryStorage(),
        location: { origin: "http://localhost" },
    });
    useSession.setState({ key: null, phase: "public", epoch: 0, storageError: null });
});
afterEach(() => {
    replaceSession(null, "public");
    vi.unstubAllGlobals();
});
describe("browser credential lifecycle", () => {
    it("keeps a rejected login candidate outside stored and authenticated state", async () => {
        vi.stubGlobal(
            "fetch",
            vi.fn(
                async () =>
                    new Response(
                        JSON.stringify({ error: { code: "unauthorized", request_id: "run.2" } }),
                        { status: 401, headers: { "Content-Type": "application/json" } },
                    ),
            ),
        );
        const epoch = replaceSession(null, "public");
        await expect(validateKey("wrong-candidate", epoch)).rejects.toMatchObject({
            code: "unauthorized",
        });
        expect(useSession.getState().phase).toBe("public");
        expect(useSession.getState().epoch).toBe(epoch);
        expect(window.localStorage.getItem(storageKey)).toBeNull();
    });
    it("persists only the credential and clears it on logout", () => {
        replaceSession("test-manager-key", "authenticated");
        expect(window.localStorage.getItem(storageKey)).toBe("test-manager-key");
        const authenticatedEpoch = useSession.getState().epoch;
        const cleared = vi.fn<() => void>();
        const dispose = onSessionReset(cleared);
        replaceSession(null, "public");
        dispose();
        expect(cleared).toHaveBeenCalledOnce();
        expect(window.localStorage.getItem(storageKey)).toBeNull();
        expect(isCurrentSession(authenticatedEpoch)).toBe(false);
        expect(useSession.getState().key).toBeNull();
    });
    it("makes unavailable browser storage a visible state", () => {
        const denied = {
            getItem: () => {
                throw new Error("denied");
            },
            setItem: () => {
                throw new Error("denied");
            },
            removeItem: () => {
                throw new Error("denied");
            },
        };
        expect(readKey(denied).storageError).not.toBeNull();
        expect(writeKey(denied, "key")).not.toBeNull();
        vi.stubGlobal("window", { localStorage: denied });
        replaceSession("key", "authenticated");
        expect(useSession.getState().storageError).not.toBeNull();
    });
    it("rejects late responses after logout even when transport ignores cancellation", async () => {
        let resolve!: (response: Response) => void;
        let requested!: () => void;
        const started = new Promise<void>((done) => {
            requested = done;
        });
        vi.stubGlobal(
            "fetch",
            vi.fn((request: Request) => {
                expect(request.credentials).toBe("omit");
                expect(request.headers.get("Authorization")).toBe("Bearer test-key");
                requested();
                return new Promise<Response>((done) => {
                    resolve = done;
                });
            }),
        );
        replaceSession("test-key", "authenticated");
        const pending = api.status();
        await started;
        replaceSession(null, "public");
        resolve(
            new Response(JSON.stringify({ data: { version: "1" } }), {
                headers: { "Content-Type": "application/json" },
            }),
        );
        await expect(pending).rejects.toMatchObject({ name: "AbortError" });
    });
    it("removes a rejected credential centrally on HTTP 401", async () => {
        vi.stubGlobal(
            "fetch",
            vi.fn(
                async () =>
                    new Response(
                        JSON.stringify({ error: { code: "unauthorized", request_id: "run.1" } }),
                        { status: 401, headers: { "Content-Type": "application/json" } },
                    ),
            ),
        );
        replaceSession("expired-key", "authenticated");
        await expect(api.status()).rejects.toMatchObject({ code: "unauthorized" });
        expect(useSession.getState().phase).toBe("public");
        expect(useSession.getState().key).toBeNull();
        expect(window.localStorage.getItem(storageKey)).toBeNull();
        expect(useSession.getState().authError).not.toBeNull();
    });
    it("does not let a late rejection clear a newer authenticated session", async () => {
        let resolve!: (response: Response) => void;
        let requested!: () => void;
        const started = new Promise<void>((done) => {
            requested = done;
        });
        vi.stubGlobal(
            "fetch",
            vi.fn(() => {
                requested();
                return new Promise<Response>((done) => {
                    resolve = done;
                });
            }),
        );
        replaceSession("old-key", "authenticated");
        const pending = api.status();
        await started;
        replaceSession("new-key", "authenticated");
        resolve(
            new Response(JSON.stringify({ error: { code: "unauthorized" } }), {
                status: 401,
                headers: { "Content-Type": "application/json" },
            }),
        );
        await expect(pending).rejects.toMatchObject({ name: "AbortError" });
        expect(useSession.getState().phase).toBe("authenticated");
        expect(useSession.getState().key).toBe("new-key");
        expect(window.localStorage.getItem(storageKey)).toBe("new-key");
    });
});
