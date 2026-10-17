import { describe, expect, it } from "vitest";
import { requestPath, safeReturnTo } from "./paths";

describe("management destinations", () => {
    it("encodes opaque request IDs as one path segment", () => {
        expect(requestPath("id/with?#%")).toBe("/requests/id%2Fwith%3F%23%25");
    });
    it("keeps local destinations and their filter context", () => {
        expect(safeReturnTo("/requests?model=example")).toBe("/requests?model=example");
        expect(safeReturnTo("/requests/example")).toBe("/requests/example");
    });
    it.each([
        undefined,
        {},
        "//example.invalid/requests",
        "https://example.invalid",
        "/v1/models",
        "/unknown",
        "/requests#fragment",
        "/requests\\private",
        "/requests\n",
    ])("rejects destinations outside the management routes: %s", (value) => {
        expect(safeReturnTo(value, "/requests")).toBe("/requests");
    });
});
