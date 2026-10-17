import { useState, type FormEvent } from "react";
import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@radix-ui/themes";
import { ApiFailure } from "../api/errors";
import { useSession } from "../session/store";
import { label, numberText, timeText } from "../lib/values";
import {
    PageTitle,
    Refresh,
    Field,
    Loading,
    Failure,
    Empty,
    Pager,
    HelpMark,
} from "../components/common";
import type { LogEntry } from "../api/generated/types.gen";
import { queries } from "../api/queries";
import { useCursor } from "../hooks/use-cursor";
import { parseSearch, logsSearchSchema, serializeSearch, logLevels } from "../routing/search";

function LogDetails({ entry }: { entry: LogEntry }) {
    return (
        <details>
            <summary>详情</summary>
            <dl className="log-metadata">
                {Object.entries(entry)
                    .filter(([key]) => !["timestamp", "level", "event", "requestId"].includes(key))
                    .map(([key, value]) => (
                        <div key={key}>
                            <dt>{key}</dt>
                            <dd>{typeof value === "string" ? value : JSON.stringify(value)}</dd>
                        </div>
                    ))}
            </dl>
        </details>
    );
}

export function Logs() {
    const [params, setParams] = useSearchParams();
    const epoch = useSession((state) => state.epoch);
    const search = parseSearch(logsSearchSchema, params);
    const [formError, setFormError] = useState<{ identity: string; message: string } | null>(null);
    const files = useQuery(queries.files(epoch, !search.error));
    const fallbackFile =
        files.data?.data.find((file) => file.active && file.fileId) ??
        files.data?.data.find((file) => file.fileId);
    const fileId = search.filters?.fileId ?? fallbackFile?.fileId ?? "";
    const selected = files.data?.data.find((file) => file.fileId === fileId);
    const minLevel = search.filters?.minLevel ?? "";
    const filters = search.filters ? { ...search.filters, fileId: fileId || undefined } : null;
    const identity = filters ? serializeSearch(filters).toString() : "";
    const paging = useCursor(identity);
    const logs = useQuery(
        queries.logs(epoch, filters, paging.cursor, !!selected?.fileId && !files.isFetching),
    );
    async function resetFiles() {
        paging.reset();
        setParams(serializeSearch({ ...search.filters, fileId: undefined }));
        await files.refetch();
    }
    function submit(event: FormEvent<HTMLFormElement>) {
        event.preventDefault();
        const next = new URLSearchParams();
        for (const [key, value] of new FormData(event.currentTarget))
            if (typeof value === "string") next.append(key, value);
        const parsed = parseSearch(logsSearchSchema, next);
        setFormError(parsed.error ? { identity, message: parsed.error } : null);
        if (parsed.filters) setParams(serializeSearch(parsed.filters));
    }
    return (
        <>
            <PageTitle
                help="每次只读取所选文件的一个窗口，由新到旧。不跨文件自动检索；服务重启、文件淘汰或截断后需重新选择。"
                action={
                    <Refresh
                        busy={files.isFetching || logs.isFetching}
                        disabled={!!search.error}
                        onClick={() => {
                            if (selected) void logs.refetch();
                            else void files.refetch();
                        }}
                    />
                }
            >
                运行日志
            </PageTitle>
            {search.error ? (
                <div role="alert" className="inline-error">
                    <p>{search.error}</p>
                    <Button variant="outline" onClick={() => setParams({})}>
                        清除筛选
                    </Button>
                </div>
            ) : files.isPending ? (
                <Loading />
            ) : files.isError ? (
                <Failure error={files.error} reset={() => void files.refetch()} />
            ) : (
                <>
                    <form className="filters" key={identity} onSubmit={submit}>
                        <Field label="日志文件">
                            <select name="fileId" defaultValue={fileId}>
                                <option value="" disabled>
                                    选择文件
                                </option>
                                {files.data.data.map((file, index) => (
                                    <option
                                        key={file.fileId ?? `unreadable-${index}`}
                                        value={file.fileId ?? `unreadable-${index}`}
                                        disabled={!file.fileId}
                                    >
                                        {file.active ? "活动文件" : "保留文件"} ·{" "}
                                        {file.state === "unreadable"
                                            ? "不可读"
                                            : timeText(file.firstTimestamp)}{" "}
                                        · {numberText(file.sizeBytes)} B
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="最低级别">
                            <select name="minLevel" defaultValue={minLevel}>
                                <option value="">全部级别</option>
                                {logLevels.map((level) => (
                                    <option key={level} value={level}>
                                        {label(level)}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="请求 ID">
                            <input
                                name="requestId"
                                defaultValue={params.get("requestId") ?? ""}
                                spellCheck={false}
                            />
                        </Field>
                        <Button variant="outline" type="submit">
                            应用筛选
                        </Button>
                        <Button
                            variant="outline"
                            type="button"
                            disabled={files.isFetching}
                            onClick={() => void files.refetch()}
                        >
                            重载文件清单
                        </Button>
                        {formError?.identity === identity && (
                            <p role="alert" className="inline-error">
                                {formError.message}
                            </p>
                        )}
                    </form>
                    {!files.data.data.length ? (
                        <Empty>没有保留的非空日志文件。</Empty>
                    ) : !fileId ? (
                        <>
                            <Empty>选择一个日志文件。</Empty>
                            {files.data.data.some((file) => file.state === "unreadable") && (
                                <p className="notice">
                                    清单中包含不可读文件；其首条完整记录无法识别。
                                </p>
                            )}
                        </>
                    ) : !selected ? (
                        <Failure
                            error={new ApiFailure("cursor_expired", 410)}
                            reset={() => void resetFiles()}
                        />
                    ) : logs.isPending ? (
                        <Loading />
                    ) : logs.isError ? (
                        <Failure
                            error={logs.error}
                            reset={() => {
                                if (logs.error instanceof ApiFailure && logs.error.status === 410)
                                    void resetFiles();
                                else void logs.refetch();
                            }}
                        />
                    ) : (
                        logs.data && (
                            <>
                                <div className="range-line">
                                    <span>
                                        {selected.active ? "活动文件" : "保留文件"} ·{" "}
                                        {numberText(selected.sizeBytes)} B
                                    </span>
                                    <span>
                                        本窗口 {logs.data.data.length} 条{" "}
                                        <HelpMark>
                                            扫描 {numberText(logs.data.scannedBytes)} 字节，跳过{" "}
                                            {logs.data.skippedRecords}{" "}
                                            条损坏或不完整记录。空窗口仍可能有下一页。
                                        </HelpMark>
                                    </span>
                                </div>
                                <section className="panel flush">
                                    <div className="table-scroll">
                                        <table className="log-table">
                                            <thead>
                                                <tr>
                                                    <th>时间</th>
                                                    <th>级别</th>
                                                    <th>事件</th>
                                                    <th>请求 ID</th>
                                                    <th>元数据</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {logs.data.data.map((entry) => (
                                                    <tr key={entry.logId}>
                                                        <td className="nowrap">
                                                            {timeText(entry.timestamp)}
                                                        </td>
                                                        <td>{label(entry.level)}</td>
                                                        <td className="mono">{entry.event}</td>
                                                        <td className="mono">
                                                            {entry.requestId ?? "—"}
                                                        </td>
                                                        <td>
                                                            <LogDetails entry={entry} />
                                                        </td>
                                                    </tr>
                                                ))}
                                            </tbody>
                                        </table>
                                    </div>
                                    {!logs.data.data.length && (
                                        <Empty>
                                            {logs.data.nextCursor
                                                ? "当前扫描窗口没有匹配记录，可继续下一页。"
                                                : "没有匹配的日志记录。"}
                                        </Empty>
                                    )}
                                </section>
                                <Pager
                                    page={paging.page}
                                    next={!!logs.data.nextCursor && paging.page < 99}
                                    previous={paging.previous}
                                    advance={() => paging.next(logs.data.nextCursor)}
                                    busy={logs.isFetching}
                                />
                                {paging.page >= 99 && (
                                    <p className="muted">
                                        已达到本次浏览页数上限，请重新选择文件或缩小筛选。
                                    </p>
                                )}
                            </>
                        )
                    )}
                </>
            )}
        </>
    );
}
