import { useState, type ReactNode, type FormEvent } from "react";
import { Button } from "@radix-ui/themes";
import { useSearchParams } from "react-router";
import { Field } from "./common";
import {
    parseSearch,
    requestsSearchSchema,
    usageSearchSchema,
    serializeSearch,
} from "../routing/search";

export function Filters({
    kind,
    extra,
    children,
}: {
    kind: "usage" | "requests";
    extra?: ReactNode;
    children?: ReactNode;
}) {
    const [params, setParams] = useSearchParams();
    const identity = params.toString();
    const [error, setError] = useState<{ identity: string; message: string } | null>(null);
    function submit(event: FormEvent<HTMLFormElement>) {
        event.preventDefault();
        const next = new URLSearchParams();
        for (const [key, value] of new FormData(event.currentTarget))
            if (typeof value === "string") next.append(key, value);
        const parsed = parseSearch(
            kind === "usage" ? usageSearchSchema : requestsSearchSchema,
            next,
        );
        setError(parsed.error ? { identity, message: parsed.error } : null);
        if (parsed.filters) setParams(serializeSearch(parsed.filters));
    }
    return (
        <form className="filters" key={identity} onSubmit={submit}>
            <Field
                label="开始时间"
                help="使用浏览器本地时区。时间范围为左闭右开，最多 31 天；留空时查询最近 24 小时。"
            >
                <input name="from" type="datetime-local" defaultValue={params.get("from") ?? ""} />
            </Field>
            <Field label="结束时间">
                <input name="to" type="datetime-local" defaultValue={params.get("to") ?? ""} />
            </Field>
            <Field label="模型">
                <input
                    name="model"
                    defaultValue={params.get("model") ?? ""}
                    placeholder="全部模型"
                />
            </Field>
            <Field label="Provider">
                <input
                    name="provider"
                    defaultValue={params.get("provider") ?? ""}
                    placeholder="全部 Provider"
                />
            </Field>
            {extra}
            <Field label="客户端流式">
                <select name="clientStream" defaultValue={params.get("clientStream") ?? ""}>
                    <option value="">全部</option>
                    <option value="true">是</option>
                    <option value="false">否</option>
                </select>
            </Field>
            <Field label="上游媒体">
                <select name="upstreamMedia" defaultValue={params.get("upstreamMedia") ?? ""}>
                    <option value="">全部</option>
                    <option value="sse">SSE</option>
                    <option value="json">JSON</option>
                    <option value="other">其他</option>
                </select>
            </Field>
            {children}
            <Button type="submit" variant="outline">
                应用筛选
            </Button>
            <Button
                type="button"
                variant="ghost"
                onClick={(event) => {
                    setError(null);
                    event.currentTarget.form?.reset();
                    setParams({});
                }}
            >
                清除
            </Button>
            {error?.identity === identity && (
                <div role="alert" className="inline-error">
                    {error.message}
                </div>
            )}
        </form>
    );
}
