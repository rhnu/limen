import { cloneElement, isValidElement, useId, type ReactNode } from "react";
import { Button, Popover, IconButton, Spinner, Tooltip } from "@radix-ui/themes";
import { CircleHelp, RefreshCw } from "lucide-react";
import { ApiFailure } from "../api/errors";

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
