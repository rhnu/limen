import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./client";
import { queries } from "./queries";
import {
    configSearchSchema,
    logsSearchSchema,
    parseSearch,
    requestsSearchSchema,
    usageSearchSchema,
} from "../routing/search";
type Api = typeof api;

vi.mock("./client", () => ({
    api: {
        status: vi.fn<Api["status"]>(),
        usage: vi.fn<Api["usage"]>(),
        requests: vi.fn<Api["requests"]>(),
        request: vi.fn<Api["request"]>(),
        files: vi.fn<Api["files"]>(),
        logs: vi.fn<Api["logs"]>(),
        config: vi.fn<Api["config"]>(),
    },
}));

function client() {
    return new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
}
const stopped = new Error("stop synthetic API call");

beforeEach(() => vi.resetAllMocks());

describe("management query options", () => {
    it("keys depend on epoch, resource and canonical filters rather than URL ordering", () => {
        const first = parseSearch(
            requestsSearchSchema,
            new URLSearchParams("state=finalized&model=%20a%20"),
        );
        const reordered = parseSearch(
            requestsSearchSchema,
            new URLSearchParams("model=a&state=finalized&provider="),
        );
        const options = queries.requests(4, first.filters);
        expect(options.queryKey).toEqual(queries.requests(4, reordered.filters).queryKey);
        expect(options.queryKey).not.toEqual(queries.requests(5, first.filters).queryKey);
        expect(options.queryKey).not.toEqual(
            queries.usage(4, parseSearch(usageSearchSchema, new URLSearchParams("model=a")).filters)
                .queryKey,
        );
        expect(queries.requests(4, first.filters, "cursor").queryKey).not.toEqual(options.queryKey);
        expect(queries.requests(4, first.filters, "cursor").queryKey).not.toEqual(
            queries.requests(
                4,
                parseSearch(requestsSearchSchema, new URLSearchParams("model=b")).filters,
                "cursor",
            ).queryKey,
        );
    });

    it("converts local datetime inputs to UTC and leaves the no-input window to the API", async () => {
        vi.mocked(api.usage).mockRejectedValue(stopped);
        const filters = parseSearch(
            usageSearchSchema,
            new URLSearchParams({ from: "2026-01-01T08:00", to: "2026-01-02T08:00" }),
        ).filters;
        await expect(client().fetchQuery(queries.usage(1, filters))).rejects.toBe(stopped);
        expect(api.usage).toHaveBeenLastCalledWith(
            {
                groupBy: "day",
                from: new Date(2026, 0, 1, 8).toISOString(),
                to: new Date(2026, 0, 2, 8).toISOString(),
            },
            expect.any(AbortSignal),
        );
        await expect(
            client().fetchQuery(
                queries.usage(1, parseSearch(usageSearchSchema, new URLSearchParams()).filters),
            ),
        ).rejects.toBe(stopped);
        expect(api.usage).toHaveBeenLastCalledWith({ groupBy: "day" }, expect.any(AbortSignal));
    });

    it("sends only cursor and limit for later request, log and config pages", async () => {
        vi.mocked(api.requests).mockRejectedValue(stopped);
        vi.mocked(api.logs).mockRejectedValue(stopped);
        vi.mocked(api.config).mockRejectedValue(stopped);
        await expect(
            client().fetchQuery(
                queries.requests(
                    1,
                    parseSearch(requestsSearchSchema, new URLSearchParams("model=a&state=pending"))
                        .filters,
                    "requests-cursor",
                ),
            ),
        ).rejects.toBe(stopped);
        await expect(
            client().fetchQuery(
                queries.logs(
                    1,
                    parseSearch(
                        logsSearchSchema,
                        new URLSearchParams("fileId=file&requestId=request&minLevel=warn"),
                    ).filters,
                    "logs-cursor",
                ),
            ),
        ).rejects.toBe(stopped);
        await expect(
            client().fetchQuery(queries.config(1, { section: "models" }, "config-cursor")),
        ).rejects.toBe(stopped);
        expect(api.requests).toHaveBeenCalledWith(
            { cursor: "requests-cursor", limit: 50 },
            expect.any(AbortSignal),
        );
        expect(api.logs).toHaveBeenCalledWith(
            { cursor: "logs-cursor", limit: 100 },
            expect.any(AbortSignal),
        );
        expect(api.config).toHaveBeenCalledWith(
            { cursor: "config-cursor", limit: 50 },
            expect.any(AbortSignal),
        );
    });

    it("omits empty optional filters and preserves selected values on first pages", async () => {
        vi.mocked(api.usage).mockRejectedValue(stopped);
        vi.mocked(api.requests).mockRejectedValue(stopped);
        vi.mocked(api.logs).mockRejectedValue(stopped);
        await expect(
            client().fetchQuery(
                queries.usage(
                    1,
                    parseSearch(usageSearchSchema, new URLSearchParams("model=&provider=")).filters,
                ),
            ),
        ).rejects.toBe(stopped);
        await expect(
            client().fetchQuery(
                queries.requests(
                    1,
                    parseSearch(
                        requestsSearchSchema,
                        new URLSearchParams("model=selected&provider=&state=&outcome=cancelled"),
                    ).filters,
                ),
            ),
        ).rejects.toBe(stopped);
        await expect(
            client().fetchQuery(
                queries.logs(
                    1,
                    parseSearch(
                        logsSearchSchema,
                        new URLSearchParams("fileId=&minLevel=warn&requestId=request"),
                    ).filters,
                ),
            ),
        ).rejects.toBe(stopped);
        expect(vi.mocked(api.usage).mock.lastCall?.[0]).toStrictEqual({ groupBy: "day" });
        expect(vi.mocked(api.requests).mock.lastCall?.[0]).toStrictEqual({
            model: "selected",
            outcome: "cancelled",
            limit: 50,
        });
        expect(vi.mocked(api.logs).mock.lastCall?.[0]).toStrictEqual({
            minLevel: "warn",
            requestId: "request",
            limit: 100,
        });
    });

    it("never calls the API for invalid filters, even if explicitly refetched", async () => {
        const invalid = parseSearch(
            usageSearchSchema,
            new URLSearchParams("groupBy=private-invalid-value"),
        );
        const options = queries.usage(1, invalid.filters);
        expect(options.enabled).toBe(false);
        await expect(client().fetchQuery(options)).rejects.toMatchObject({ code: "invalid_query" });
        expect(api.usage).not.toHaveBeenCalled();
        expect(options.queryKey).not.toContain("private-invalid-value");
        expect(queries.status(1, false).enabled).toBe(false);
        expect(queries.logs(1, null).enabled).toBe(false);
        expect(queries.files(1, false).enabled).toBe(false);
        const request = queries.request(1, null);
        expect(request.enabled).toBe(false);
        await expect(client().fetchQuery(request)).rejects.toMatchObject({ code: "invalid_query" });
        expect(api.request).not.toHaveBeenCalled();
    });

    it("supplies abort signals for every query resource", async () => {
        vi.mocked(api.status).mockRejectedValue(stopped);
        vi.mocked(api.request).mockRejectedValue(stopped);
        vi.mocked(api.files).mockRejectedValue(stopped);
        await expect(client().fetchQuery(queries.status(2))).rejects.toBe(stopped);
        await expect(client().fetchQuery(queries.request(2, "request-id"))).rejects.toBe(stopped);
        await expect(client().fetchQuery(queries.files(2))).rejects.toBe(stopped);
        expect(api.status).toHaveBeenCalledWith(expect.any(AbortSignal));
        expect(api.request).toHaveBeenCalledWith("request-id", expect.any(AbortSignal));
        expect(api.files).toHaveBeenCalledWith(expect.any(AbortSignal));
    });

    it("passes React Query's abort signal through to the API", async () => {
        let signal: AbortSignal | undefined;
        vi.mocked(api.requests).mockImplementation((_query, requestSignal) => {
            signal = requestSignal;
            return new Promise((_resolve, reject) => {
                requestSignal?.addEventListener("abort", () =>
                    reject(new DOMException("Aborted", "AbortError")),
                );
            });
        });
        const queryClient = client();
        const options = queries.requests(
            1,
            parseSearch(requestsSearchSchema, new URLSearchParams()).filters,
        );
        const result = queryClient.fetchQuery(options).catch((error: unknown) => error);
        expect(signal).toBeInstanceOf(AbortSignal);
        expect(signal?.aborted).toBe(false);
        await queryClient.cancelQueries({ queryKey: options.queryKey });
        expect(signal?.aborted).toBe(true);
        expect(await result).toBeInstanceOf(Error);
    });

    it("keeps a log scan's empty page and nonempty next cursor", async () => {
        const page = {
            data: [],
            fileId: "file",
            nextCursor: "more-scan",
            scannedBytes: "1024",
            skippedRecords: 0,
        };
        vi.mocked(api.logs).mockResolvedValue(page);
        expect(await client().fetchQuery(queries.logs(1, { fileId: "file" }))).toEqual(page);
    });

    it("narrows config pages by the requested section, including empty arrays", async () => {
        vi.mocked(api.config).mockResolvedValue({ data: [], nextCursor: null });
        const models = parseSearch(
            configSearchSchema,
            new URLSearchParams("section=models"),
        ).filters;
        expect(await client().fetchQuery(queries.config(1, models))).toEqual({
            section: "models",
            data: [],
            nextCursor: null,
        });
        expect(await client().fetchQuery(queries.config(1, { section: "providers" }))).toEqual({
            section: "providers",
            data: [],
            nextCursor: null,
        });
        expect(api.config).toHaveBeenLastCalledWith(
            { section: "providers", limit: 50 },
            expect.any(AbortSignal),
        );
        vi.mocked(api.config).mockResolvedValue({
            data: [{ id: "provider", origin: "https://example.test" }],
            nextCursor: null,
        });
        await expect(client().fetchQuery(queries.config(1, models))).rejects.toMatchObject({
            code: "invalid_response",
        });
    });
});
