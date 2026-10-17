import type { FormEvent } from "react";
import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@radix-ui/themes";
import { api, ApiFailure } from "../api/client";
import { useSession } from "../lib/session";
import { label, numberText, timeText } from "../lib/values";
import {
    PageTitle,
    Refresh,
    Field,
    Loading,
    Failure,
    Empty,
    Pager,
    useCursor,
    HelpMark,
} from "../components/common";
import type { GetLogsData, LogEntry, LogLevel } from "../api/generated/types.gen";

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
                            <dd>
                                {typeof value === "object" ? JSON.stringify(value) : String(value)}
                            </dd>
                        </div>
                    ))}
            </dl>
        </details>
    );
}

export function Logs() {
    const [params, setParams] = useSearchParams();
    const epoch = useSession((state) => state.epoch);
    const files = useQuery({
        queryKey: [epoch, "log-files"],
        queryFn: ({ signal }) => api.files(signal),
    });
    const fallbackFile =
        files.data?.data.find((file) => file.active && file.fileId) ??
        files.data?.data.find((file) => file.fileId);
    const fileId = params.get("fileId") || fallbackFile?.fileId || "";
    const selected = files.data?.data.find((file) => file.fileId === fileId);
    const minLevel = params.get("minLevel") || "";
    const paging = useCursor(`${fileId}:${params.toString()}`);
    const query: GetLogsData["query"] = paging.cursor
        ? { cursor: paging.cursor, limit: 100 }
        : {
              fileId,
              limit: 100,
              requestId: params.get("requestId") || undefined,
              minLevel: ["trace", "debug", "info", "warn", "error"].includes(minLevel)
                  ? (minLevel as LogLevel)
                  : undefined,
          };
    const logs = useQuery({
        queryKey: [epoch, "logs", fileId, params.toString(), paging.cursor],
        queryFn: ({ signal }) => api.logs(query, signal),
        enabled: !!selected?.fileId && !files.isFetching,
    });
    async function resetFiles() {
        paging.reset();
        const next = new URLSearchParams(params);
        next.delete("fileId");
        setParams(next);
        await files.refetch();
    }
    function submit(event: FormEvent<HTMLFormElement>) {
        event.preventDefault();
        const form = new FormData(event.currentTarget);
        const next = new URLSearchParams();
        for (const [key, value] of form)
            if (String(value).trim()) next.set(key, String(value).trim());
        setParams(next);
    }
    return (
        <>
            <PageTitle
                help="每次只读取所选文件的一个窗口，由新到旧。不跨文件自动检索；服务重启、文件淘汰或截断后需重新选择。"
                action={
                    <Refresh
                        busy={files.isFetching || logs.isFetching}
                        onClick={() => {
                            if (selected) void logs.refetch();
                            else void files.refetch();
                        }}
                    />
                }
            >
                运行日志
            </PageTitle>
            {files.isPending ? (
                <Loading />
            ) : files.isError ? (
                <Failure error={files.error} reset={() => void files.refetch()} />
            ) : (
                <>
                    <form
                        className="filters"
                        key={`${fileId}:${params.toString()}`}
                        onSubmit={submit}
                    >
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
                                {["trace", "debug", "info", "warn", "error"].map((level) => (
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
