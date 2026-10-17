import * as v from "valibot";
import type { GroupBy, LogLevel, Outcome, RecordState, Section } from "../api/generated/types.gen";
import { windowError } from "../lib/values";

export const recordStates = ["pending", "finalized", "interrupted"] satisfies RecordState[];
export const outcomes = ["success", "failed", "cancelled"] satisfies Outcome[];
export const logLevels = ["trace", "debug", "info", "warn", "error"] satisfies LogLevel[];
const groups = ["day", "model", "provider", "none"] satisfies GroupBy[];
const sections = ["server", "models", "providers"] satisfies Section[];
const invalidQuery = "查询参数无效，请清除或重新应用筛选。";
export const opaqueIdSchema = v.pipe(
    v.string(invalidQuery),
    v.regex(/^[A-Za-z0-9-]{1,128}$/, invalidQuery),
);

export function parseRequestId(value: unknown) {
    const result = v.safeParse(opaqueIdSchema, value);
    return result.success
        ? { id: result.output, error: null }
        : { id: null, error: "请求 ID 无效。" };
}

// datetime-local values belong to the browser's local timezone. Check components as
// well as Date.parse: Date otherwise silently rolls invalid calendar dates forward.
function localDate(value: string): Date | null {
    const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?$/.exec(
        value,
    );
    if (!match) return null;
    const [, year, month, day, hour, minute, second = "0", fraction = "0"] = match;
    const parts = [
        Number(year),
        Number(month) - 1,
        Number(day),
        Number(hour),
        Number(minute),
        Number(second),
        Number(fraction.padEnd(3, "0")),
    ];
    const date = new Date(value);
    const actual = [
        date.getFullYear(),
        date.getMonth(),
        date.getDate(),
        date.getHours(),
        date.getMinutes(),
        date.getSeconds(),
        date.getMilliseconds(),
    ];
    return Number(year) > 0 && actual.every((part, index) => part === parts[index]) ? date : null;
}

function normalizeLocalDate(value: string): string {
    if (!value) return value;
    // Drop equivalent zero seconds/fractions so they share a query identity.
    return value
        .replace(/(\.\d*?)0+$/, "$1")
        .replace(/\.$/, "")
        .replace(/(T\d{2}:\d{2}):00$/, "$1");
}

const dateField = v.optional(
    v.pipe(
        v.string(invalidQuery),
        v.check((value) => value === "" || localDate(value) !== null, "时间无效，请重新选择。"),
        v.transform(normalizeLocalDate),
    ),
    "",
);
const textField = v.optional(
    v.pipe(
        v.string(invalidQuery),
        v.trim(),
        v.check((value) => new TextEncoder().encode(value).length <= 128, invalidQuery),
        v.transform((value) => value || undefined),
    ),
);
const idField = v.optional(
    v.pipe(
        v.string(invalidQuery),
        v.trim(),
        v.union([v.literal(""), opaqueIdSchema], invalidQuery),
        v.transform((value) => value || undefined),
    ),
);
function optionalEnum<const T extends readonly string[]>(values: T) {
    return v.optional(
        v.pipe(
            v.union([v.literal(""), v.picklist(values, invalidQuery)], invalidQuery),
            v.transform((value) => (value === "" ? undefined : value)),
        ),
    );
}
const windowFields = { from: dateField, to: dateField, model: textField, provider: textField };
const windowMessage = "请同时提供开始和结束时间，结束时间须更晚，且范围最多为 31 天。";

export const usageSearchSchema = v.pipe(
    v.strictObject(
        {
            ...windowFields,
            groupBy: v.optional(
                v.pipe(
                    v.union([v.literal(""), v.picklist(groups, invalidQuery)], invalidQuery),
                    v.transform((value) => (value === "" ? "day" : value)),
                ),
                "day",
            ),
        },
        invalidQuery,
    ),
    v.check((value) => windowError(value.from, value.to) === null, windowMessage),
);
export const requestsSearchSchema = v.pipe(
    v.strictObject(
        {
            ...windowFields,
            state: optionalEnum(recordStates),
            outcome: optionalEnum(outcomes),
        },
        invalidQuery,
    ),
    v.check((value) => windowError(value.from, value.to) === null, windowMessage),
);
export const logsSearchSchema = v.strictObject(
    {
        fileId: idField,
        minLevel: optionalEnum(logLevels),
        requestId: idField,
    },
    invalidQuery,
);
export const configSearchSchema = v.strictObject(
    {
        section: v.optional(
            v.pipe(
                v.union([v.literal(""), v.picklist(sections, invalidQuery)], invalidQuery),
                v.transform((value) => (value === "" ? "server" : value)),
            ),
            "server",
        ),
    },
    invalidQuery,
);
export const emptySearchSchema = v.strictObject({}, invalidQuery);

export type UsageSearch = v.InferOutput<typeof usageSearchSchema>;
export type RequestsSearch = v.InferOutput<typeof requestsSearchSchema>;
export type LogsSearch = v.InferOutput<typeof logsSearchSchema>;
export type ConfigSearch = v.InferOutput<typeof configSearchSchema>;

export function serializeSearch(filters: Record<string, string | undefined>): URLSearchParams {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(filters).sort(([left], [right]) =>
        left.localeCompare(right),
    )) {
        if (value !== undefined && value !== "") params.set(key, value);
    }
    return params;
}

export function parseSearch<S extends v.GenericSchema<unknown, Record<string, string | undefined>>>(
    schema: S,
    params: URLSearchParams,
) {
    if (new TextEncoder().encode(params.toString()).length > 4096)
        return { filters: null, error: invalidQuery, canonical: "" };
    const entries: Record<string, string> = {};
    for (const [key, value] of params) {
        if (Object.hasOwn(entries, key))
            return { filters: null, error: "查询参数不能重复。", canonical: "" };
        Object.defineProperty(entries, key, { value, enumerable: true });
    }
    const parsed = v.safeParse(schema, entries);
    if (!parsed.success) return { filters: null, error: parsed.issues[0].message, canonical: "" };
    return {
        filters: parsed.output,
        error: null,
        canonical: serializeSearch(parsed.output).toString(),
    };
}
