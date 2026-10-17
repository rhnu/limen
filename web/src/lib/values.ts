import type { TokenSum, UsageGroup } from "../api/generated/types.gen";

export function sumTokens(values: TokenSum[]): TokenSum {
    let sum: bigint | null = null;
    let samples = 0;
    for (const value of values) {
        if (value.sum !== null) sum = (sum ?? 0n) + BigInt(value.sum);
        samples += value.samples;
    }
    return { sum: sum?.toString() ?? null, samples };
}

export function usageTotals(groups: UsageGroup[]) {
    return {
        records: groups.reduce((total, group) => total + BigInt(group.records), 0n),
        input: sumTokens(groups.map((group) => group.tokens.input)),
        output: sumTokens(groups.map((group) => group.tokens.output)),
        total: sumTokens(groups.map((group) => group.tokens.total)),
    };
}

export function numberText(value: string | number | bigint | null | undefined): string {
    if (value === null || value === undefined) return "未知";
    return typeof value === "string"
        ? BigInt(value).toLocaleString("zh-CN")
        : value.toLocaleString("zh-CN");
}

export function timeText(value: string | null | undefined): string {
    return value
        ? `${new Date(value).toLocaleString("zh-CN", { hour12: false, timeZone: "UTC" })} UTC`
        : "未知";
}

export function windowError(from: string, to: string): string | null {
    if (!from && !to) return null;
    if (!from || !to) return "请同时选择开始和结束时间。";
    const start = Date.parse(from);
    const end = Date.parse(to);
    if (!Number.isFinite(start) || !Number.isFinite(end)) return "时间无效。";
    if (end <= start) return "结束时间须晚于开始时间。";
    if (end - start > 31 * 24 * 60 * 60 * 1000) return "时间范围最多为 31 天。";
    return null;
}

export function dateQuery(from: string, to: string): { from?: string; to?: string } {
    return from && to ? { from: new Date(from).toISOString(), to: new Date(to).toISOString() } : {};
}

const labels: Record<string, string> = {
    pending: "进行中",
    finalized: "已终结",
    interrupted: "已中断",
    success: "成功",
    failed: "失败",
    cancelled: "已取消",
    missing: "缺失",
    partial: "部分",
    reported: "已报告",
    invalid: "无效",
    conflicting: "冲突",
    ready: "就绪",
    stopping: "停止中",
    running: "运行中",
    retired: "已退役",
    true: "是",
    false: "否",
    unknown: "未知",
    none: "无",
    trace: "追踪",
    debug: "调试",
    info: "信息",
    warn: "警告",
    error: "错误",
    day: "日期",
    model: "模型",
    provider: "Provider",
};
export function label(value: string | boolean | null | undefined): string {
    return value === null || value === undefined
        ? "未知"
        : (labels[String(value)] ?? String(value));
}

export function safeChartValue(value: string | null): number | null {
    if (value === null) return null;
    const count = BigInt(value);
    return count <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(count) : null;
}
