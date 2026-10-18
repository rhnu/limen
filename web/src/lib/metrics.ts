import type {
    DownstreamMode,
    GenerationParameters,
    GenerationFacts,
    MetricStatus,
    RequestMetrics,
    RequestTimings,
} from "../api/generated/types.gen";

export const metricReasons: Record<MetricStatus, string> = {
    available: "可计算",
    not_reached: "未达到",
    not_sse: "不适用：上游非 SSE",
    incomplete: "响应不完整",
    missing_usage: "缺少输出用量",
    invalid_usage: "用量无效",
    multiple_choices: "多 choice 不可估算",
    insufficient_tokens: "输出词元不足",
    insufficient_observations: "独立生成观测不足",
    zero_duration: "计时间隔为零",
    token_scope_mismatch: "词元与生成范围不一致",
};
export const downstreamModes: Record<DownstreamMode, string> = {
    native_stream: "原生流交付",
    buffered: "缓冲交付",
    not_streamed: "非流式交付",
    not_reached: "未交付",
};
export function decimalText(value: number | null, unit = ""): string {
    return value === null
        ? "未知"
        : `${value.toLocaleString("zh-CN", { maximumFractionDigits: 3 })}${unit ? ` ${unit}` : ""}`;
}
export const microsecondsText = (value: number | null) =>
    decimalText(value === null ? null : value / 1000, "ms");
export function tpsText(value: number | null, status: MetricStatus): string {
    return value === null ? metricReasons[status] : decimalText(value, "token/s");
}
export function ttftText(
    metrics: RequestMetrics,
    side: "upstream" | "downstream",
    upstreamMedia: string | null = null,
): string {
    if (side === "downstream" && metrics.downstreamMode === "not_streamed") return "不适用：非流式";
    const value = side === "upstream" ? metrics.upstreamTtftMs : metrics.downstreamTtftMs;
    if (value !== null) return decimalText(value, "ms");
    if (side === "downstream") return "未达到";
    return upstreamMedia === null
        ? "未知"
        : upstreamMedia === "sse"
          ? "未达到"
          : "不适用：上游非 SSE";
}
export const timingFields: { key: keyof RequestTimings; title: string; help: string }[] = [
    { key: "requestUs", title: "请求总耗时", help: "生成管线终结前的总耗时，不含清理。" },
    { key: "requestDecodeUs", title: "请求解析", help: "解析和校验客户端 JSON。" },
    { key: "storageBeginUs", title: "持久化准入", help: "等待初始记录确认。" },
    {
        key: "preSendUs",
        title: "发送前准备",
        help: "请求开始至准备发送上游，包含其中的解析、持久化与策略。",
    },
    { key: "routingUs", title: "路由", help: "一次路由选择。" },
    { key: "requestEncodeUs", title: "上游编码", help: "最终上游请求编码。" },
    { key: "upstreamHeadersWaitUs", title: "上游响应头等待", help: "发送上游至收到响应头。" },
    { key: "upstreamFirstByteUs", title: "上游首字节", help: "发送上游至读取到首个正文数据。" },
    {
        key: "upstreamFirstChunkUs",
        title: "上游首完整事件",
        help: "发送上游至解析到首个完整事件，可能仅有角色或元数据。",
    },
    { key: "upstreamTotalUs", title: "上游总耗时", help: "发送上游至上游处理结束。" },
    {
        key: "downstreamFirstEventUs",
        title: "下游首事件",
        help: "请求开始至交付首事件，可能不包含生成内容。",
    },
    { key: "upstreamReadWaitUs", title: "上游读取等待", help: "正文读取等待累计值。" },
    { key: "downstreamQueueWaitUs", title: "下游队列等待", help: "有界输出队列背压等待累计值。" },
    { key: "downstreamDrainUs", title: "下游排空", help: "输出完成后的有界排空等待。" },
    { key: "cleanupUs", title: "终结清理", help: "终结观察和错误清理耗时。" },
];
export function parameterRows(client: GenerationParameters, upstream: GenerationParameters | null) {
    return [...new Set([...Object.keys(client), ...Object.keys(upstream ?? {})])]
        .sort()
        .map((key) => ({
            key,
            client: Object.hasOwn(client, key) ? JSON.stringify(client[key]) : "未记录",
            upstream:
                upstream === null
                    ? "未发送"
                    : Object.hasOwn(upstream, key)
                      ? JSON.stringify(upstream[key])
                      : "未记录",
        }));
}

export function transportText(clientStream: boolean, upstreamMedia: string | null): string {
    if (upstreamMedia === "json")
        return clientStream ? "JSON → SSE（缓冲合成）" : "JSON → JSON（原生 JSON）";
    if (upstreamMedia === "sse") return clientStream ? "SSE → SSE（原生流）" : "SSE → JSON（聚合）";
    return upstreamMedia === null ? "未知" : "其他上游媒体";
}
export function firstTextText(
    value: number | null,
    media: string | null,
    nativeOnly = false,
): string {
    if (value !== null) return decimalText(value, "ms");
    return media === null
        ? "未知"
        : nativeOnly && media !== "sse"
          ? "不适用：上游非 SSE"
          : "未达到";
}

export function finishReasonText(
    facts: Pick<
        GenerationFacts,
        "finishStop" | "finishLength" | "finishTool" | "finishFiltered" | "finishOther"
    >,
    completed: boolean | null,
): string {
    const counts: [string, number][] = [
        ["正常停止", facts.finishStop],
        ["长度上限", facts.finishLength],
        ["工具调用", facts.finishTool],
        ["内容过滤", facts.finishFiltered],
        ["其他", facts.finishOther],
    ];
    const observed = counts.filter(([, count]) => count > 0);
    if (!observed.length)
        return completed === false
            ? "未完成，未观察到结束原因"
            : completed === null
              ? "完成状态未知，无结束原因样本"
              : "无结束原因样本";
    const categories = observed.map(([title, count]) => `${title} ${count}`).join(" · ");
    return (
        categories +
        (completed === false ? "；上游未完成" : completed === null ? "；上游完成状态未知" : "")
    );
}
