import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { queryClient } from "./query-client";
import { replaceSession } from "../session/store";

beforeEach(() => {
    vi.stubGlobal("window", { localStorage: { setItem: () => {}, removeItem: () => {} } });
});
afterEach(() => {
    replaceSession(null, "public");
    vi.unstubAllGlobals();
});

describe("session query cleanup", () => {
    it("cancels an active query and removes loaded data when credentials change", async () => {
        replaceSession("old-key", "authenticated");
        queryClient.setQueryData(["loaded"], { records: 2 });
        let signal: AbortSignal | undefined;
        const pending = queryClient.fetchQuery({
            queryKey: ["active"],
            queryFn: (context) => {
                signal = context.signal;
                return new Promise<never>(() => {});
            },
        });
        replaceSession("new-key", "authenticated");
        await expect(pending).rejects.toBeDefined();
        expect(signal?.aborted).toBe(true);
        expect(queryClient.getQueryData(["loaded"])).toBeUndefined();
        expect(queryClient.getQueryCache().getAll()).toHaveLength(0);
    });
});
