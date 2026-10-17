import {
    cloneElement,
    isValidElement,
    useId,
    useState,
    type ReactNode,
    type FormEvent,
} from "react";
import { Button, Popover, IconButton, Spinner, Tooltip } from "@radix-ui/themes";
import { CircleHelp, RefreshCw } from "lucide-react";
import { useSearchParams } from "react-router";
import { windowError } from "../lib/values";
import { ApiFailure } from "../api/client";

export function HelpMark({
    children,
    label = "查看说明",
}: {
    children: ReactNode;
    label?: string;
}) {
    return (
        <Popover.Root>
            <Tooltip content={children} delayDuration={250}>
                <Popover.Trigger>
                    <IconButton
                        className="help-mark"
                        size="1"
                        variant="ghost"
                        aria-label={label}
                        type="button"
                    >
                        <CircleHelp size={15} aria-hidden="true" />
                    </IconButton>
                </Popover.Trigger>
            </Tooltip>
            <Popover.Content size="2" maxWidth="350px">
                <div className="help-content">{children}</div>
                <Popover.Close>
                    <Button variant="soft" size="1" mt="3">
                        关闭
                    </Button>
                </Popover.Close>
            </Popover.Content>
        </Popover.Root>
    );
}

export function PageTitle({
    children,
    help,
    action,
}: {
    children: ReactNode;
    help?: ReactNode;
    action?: ReactNode;
}) {
    return (
        <div className="page-title">
            <div className="heading-with-help">
                <h1>{children}</h1>
                {help && <HelpMark>{help}</HelpMark>}
            </div>
            {action}
        </div>
    );
}
export function Refresh({
    onClick,
    busy,
    disabled = false,
}: {
    onClick: () => void;
    busy: boolean;
    disabled?: boolean;
}) {
    return (
        <Button variant="outline" onClick={onClick} disabled={busy || disabled}>
            <RefreshCw size={15} aria-hidden="true" />
            刷新
        </Button>
    );
}
export function Loading() {
    return (
        <div className="state" role="status">
            <Spinner />
            正在加载…
        </div>
    );
}
export function Empty({ children = "没有符合条件的记录。" }: { children?: ReactNode }) {
    return <div className="state muted">{children}</div>;
}
export function Failure({ error, reset }: { error: unknown; reset?: () => void }) {
    const failure = error instanceof ApiFailure ? error : new ApiFailure("network_error");
    return (
        <div role="alert" className="error-state">
            <strong>{failure.message}</strong>
            <div className="error-meta">
                {failure.code}
                {failure.requestId && (
                    <>
                        {" "}
                        · 请求 ID：<code>{failure.requestId}</code>
                    </>
                )}
            </div>
            {reset && (
                <Button variant="outline" onClick={reset} mt="3">
                    {failure.status === 410 ? "重置查询" : "重试"}
                </Button>
            )}
        </div>
    );
}
export function Field({
    label,
    help,
    children,
}: {
    label: string;
    help?: ReactNode;
    children: ReactNode;
}) {
    const id = useId();
    return (
        <div className="field">
            <span className="field-label">
                <label htmlFor={id}>{label}</label>
                {help && <HelpMark>{help}</HelpMark>}
            </span>
            {isValidElement<{ id?: string }>(children) ? cloneElement(children, { id }) : children}
        </div>
    );
}
export function Pager({
    page,
    next,
    previous,
    advance,
    busy,
}: {
    page: number;
    next: boolean;
    previous: () => void;
    advance: () => void;
    busy: boolean;
}) {
    return (
        <div className="pager">
            <span className="muted">第 {page + 1} 页</span>
            <Button variant="outline" disabled={!page || busy} onClick={previous}>
                上一页
            </Button>
            <Button variant="outline" disabled={!next || busy} onClick={advance}>
                下一页
            </Button>
        </div>
    );
}
export function useCursor(identity: string) {
    const [state, setState] = useState<{ identity: string; cursors: (string | undefined)[] }>({
        identity,
        cursors: [undefined],
    });
    if (state.identity !== identity) setState({ identity, cursors: [undefined] });
    const cursors = state.identity === identity ? state.cursors : [undefined];
    return {
        cursor: cursors[cursors.length - 1],
        page: cursors.length - 1,
        next: (cursor: string | null | undefined) => {
            if (cursor && cursors.length < 100)
                setState({ identity, cursors: [...cursors, cursor] });
        },
        previous: () =>
            setState({ identity, cursors: cursors.length > 1 ? cursors.slice(0, -1) : cursors }),
        reset: () => setState({ identity, cursors: [undefined] }),
    };
}

export function Filters({ extra, children }: { extra?: ReactNode; children?: ReactNode }) {
    const [params, setParams] = useSearchParams();
    const identity = params.toString();
    const [error, setError] = useState<{ identity: string; message: string } | null>(null);
    function submit(event: FormEvent<HTMLFormElement>) {
        event.preventDefault();
        const form = new FormData(event.currentTarget);
        const from = String(form.get("from") ?? "");
        const to = String(form.get("to") ?? "");
        const invalid = windowError(from, to);
        setError(invalid ? { identity, message: invalid } : null);
        if (invalid) return;
        const next = new URLSearchParams();
        for (const [key, value] of form)
            if (String(value).trim()) next.set(key, String(value).trim());
        setParams(next);
    }
    return (
        <form className="filters" key={params.toString()} onSubmit={submit}>
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
