import { describe, expect, it } from "vitest";
import type { RequestMetrics } from "../api/generated/types.gen";
import {
    decimalText,
    microsecondsText,
    parameterRows,
    tpsText,
    ttftText,
    transportText,
    firstTextText,
    finishReasonText,
} from "./metrics";
const metrics: RequestMetrics = {
    upstreamTtftMs: null,
    downstreamTtftMs: null,
    upstreamFirstTextMs: null,
    downstreamFirstTextMs: null,
    averageOutputTps: null,
    averageOutputTpsStatus: "missing_usage",
    generationTps: null,
    generationTpsStatus: "not_sse",
    downstreamMode: "not_reached",
};
describe("performance formatting", () => {
    it("preserves real zero and sub-millisecond measurements", () => {
        expect(decimalText(0, "ms")).toBe("0 ms");
        expect(microsecondsText(25)).toBe("0.025 ms");
        expect(microsecondsText(null)).toBe("未知");
        expect(tpsText(0, "available")).toBe("0 token/s");
        expect(ttftText({ ...metrics, downstreamTtftMs: 0 }, "downstream")).toBe("0 ms");
    });
    it("distinguishes not reached, unknown, non-stream and calculation reasons", () => {
        expect(ttftText(metrics, "upstream")).toBe("未知");
        expect(ttftText(metrics, "upstream", "sse")).toBe("未达到");
        expect(ttftText(metrics, "upstream", "json")).toBe("不适用：上游非 SSE");
        expect(ttftText(metrics, "downstream")).toBe("未达到");
        expect(
            ttftText(
                { ...metrics, downstreamMode: "not_streamed", downstreamTtftMs: 100 },
                "downstream",
            ),
        ).toBe("不适用：非流式");
        expect(tpsText(null, "incomplete")).toBe("响应不完整");
        expect(tpsText(null, "missing_usage")).toBe("缺少输出用量");
        expect(tpsText(null, "not_sse")).toBe("不适用：上游非 SSE");
    });
    it("keeps buffered delivery visibly separate from native streaming", () => {
        const buffered = {
            ...metrics,
            downstreamMode: "buffered" as const,
            downstreamTtftMs: 200.125,
        };
        expect(ttftText(buffered, "downstream")).toBe("200.125 ms");
    });
});
describe("generation parameter comparison", () => {
    it("aligns recorded keys and preserves false, zero, null and safe nested summaries", () => {
        expect(
            parameterRows(
                { temperature: 0, stream: false, metadata: null },
                { temperature: 1, summary: { count: 2 } },
            ),
        ).toEqual([
            { key: "metadata", client: "null", upstream: "未记录" },
            { key: "stream", client: "false", upstream: "未记录" },
            { key: "summary", client: "未记录", upstream: '{"count":2}' },
            { key: "temperature", client: "0", upstream: "1" },
        ]);
    });
    it("distinguishes no upstream send from omitted upstream parameters", () => {
        expect(parameterRows({ stream: true }, null)).toEqual([
            { key: "stream", client: "true", upstream: "未发送" },
        ]);
        expect(parameterRows({}, {})).toEqual([]);
    });
});

describe("transport labels", () => {
    it("names all four JSON and SSE combinations without claiming native timings for buffered output", () => {
        expect(transportText(true, "json")).toBe("JSON → SSE（缓冲合成）");
        expect(transportText(false, "json")).toBe("JSON → JSON（原生 JSON）");
        expect(transportText(true, "sse")).toBe("SSE → SSE（原生流）");
        expect(transportText(false, "sse")).toBe("SSE → JSON（聚合）");
        expect(transportText(true, null)).toBe("未知");
        expect(firstTextText(0, "sse", true)).toBe("0 ms");
        expect(firstTextText(null, "sse", true)).toBe("未达到");
        expect(firstTextText(null, "json", true)).toBe("不适用：上游非 SSE");
    });
});

describe("finish reason categories", () => {
    const empty = {
        finishStop: 0,
        finishLength: 0,
        finishTool: 0,
        finishFiltered: 0,
        finishOther: 0,
    };
    it("distinguishes no samples, incomplete and unknown completion", () => {
        expect(finishReasonText(empty, true)).toBe("无结束原因样本");
        expect(finishReasonText(empty, false)).toBe("未完成，未观察到结束原因");
        expect(finishReasonText(empty, null)).toBe("完成状态未知，无结束原因样本");
    });
    it("shows only observed fixed categories without inferring completion", () => {
        expect(finishReasonText({ ...empty, finishStop: 2, finishTool: 1 }, true)).toBe(
            "正常停止 2 · 工具调用 1",
        );
        expect(
            finishReasonText(
                { ...empty, finishLength: 1, finishFiltered: 1, finishOther: 2 },
                false,
            ),
        ).toBe("长度上限 1 · 内容过滤 1 · 其他 2；上游未完成");
    });
});
