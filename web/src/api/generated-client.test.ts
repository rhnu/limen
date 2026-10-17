import { describe, expect, it, vi } from "vitest";
import { createClient } from "./generated/client";
import type { StreamEvent } from "./generated/core/serverSentEvents.gen";

describe("generated Fetch client", () => {
    it.each([undefined, null, { message: "sample" }])(
        "preserves SSE requests and events with body %j",
        async (body) => {
            const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
                new Response('data: {"ok":true}\n\nevent: done\nid: 7\ndata: finish\n\n', {
                    headers: { "Content-Type": "text/event-stream" },
                }),
            );
            const client = createClient({ baseUrl: "https://example.test", fetch });
            const events: StreamEvent[] = [];
            const errors: unknown[] = [];
            const options = {
                url: "/events",
                headers: { "X-Test": "synthetic" },
                sseMaxRetryAttempts: 1,
                onSseEvent: (event: StreamEvent) => events.push(event),
                onSseError: (error: unknown) => errors.push(error),
            };
            const result =
                body === undefined
                    ? await client.sse.get(options)
                    : await client.sse.post({ ...options, body });
            const values = [];
            for await (const value of result.stream) values.push(value);

            expect(errors).toStrictEqual([]);
            expect(fetch).toHaveBeenCalledOnce();
            const request = fetch.mock.lastCall?.[0];
            if (!(request instanceof Request)) throw new Error("Expected a Fetch Request");
            expect(request.url).toBe("https://example.test/events");
            expect(request.method).toBe(body === undefined ? "GET" : "POST");
            expect(request.headers.get("X-Test")).toBe("synthetic");
            expect(await request.text()).toBe(body === undefined ? "" : JSON.stringify(body));
            expect(values).toStrictEqual([{ ok: true }, "finish"]);
            expect(events).toMatchObject([
                { data: { ok: true }, event: undefined, id: undefined },
                { data: "finish", event: "done", id: "7" },
            ]);
        },
    );
});
