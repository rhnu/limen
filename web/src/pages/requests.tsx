import { Link, useLocation, useParams, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ArrowUpRight } from "lucide-react";
import { api, ApiFailure } from "../api/client";
import { useSession } from "../lib/session";
import { dateQuery, label, numberText, timeText, windowError } from "../lib/values";
import {
    PageTitle,
    Refresh,
    Filters,
    Field,
    Loading,
    Failure,
    Empty,
    Pager,
    useCursor,
    HelpMark,
} from "../components/common";
import type { GetRequestsData, Outcome, RecordState } from "../api/generated/types.gen";
import type { ReactNode } from "react";

export function Requests() {
    const [params] = useSearchParams();
    const location = useLocation();
    const epoch = useSession((state) => state.epoch);
    const paging = useCursor(params.toString());
    const from = params.get("from") ?? "";
    const to = params.get("to") ?? "";
    const invalid = windowError(from, to);
    const state = params.get("state");
    const outcome = params.get("outcome");
    const query: GetRequestsData["query"] = paging.cursor
        ? { cursor: paging.cursor, limit: 50 }
        : {
              ...(!invalid ? dateQuery(from, to) : {}),
              model: params.get("model") || undefined,
              provider: params.get("provider") || undefined,
              state: ["pending", "finalized", "interrupted"].includes(state ?? "")
                  ? (state as RecordState)
                  : undefined,
              outcome: ["success", "failed", "cancelled"].includes(outcome ?? "")
                  ? (outcome as Outcome)
                  : undefined,
              limit: 50,
          };
    const result = useQuery({
        queryKey: [epoch, "requests", params.toString(), paging.cursor],
        queryFn: ({ signal }) => api.requests(query, signal),
        enabled: !invalid,
    });
    return (
        <>
            <PageTitle
                help="按开始时间倒序。已终结表示网关管线结束，不证明客户端收到全部内容。分页期间终态和保留清理仍可能变化。"
                action={
                    <Refresh
                        busy={result.isFetching}
                        disabled={!!invalid}
                        onClick={() => void result.refetch()}
                    />
                }
            >
                请求记录
            </PageTitle>
            <Filters
                extra={
                    <>
                        <Field label="状态">
                            <select name="state" defaultValue={state ?? ""}>
                                <option value="">全部状态</option>
                                {["pending", "finalized", "interrupted"].map((value) => (
                                    <option key={value} value={value}>
                                        {label(value)}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="结果">
                            <select name="outcome" defaultValue={outcome ?? ""}>
                                <option value="">全部结果</option>
                                {["success", "failed", "cancelled"].map((value) => (
                                    <option key={value} value={value}>
                                        {label(value)}
                                    </option>
                                ))}
                            </select>
                        </Field>
                    </>
                }
            />
            {invalid ? (
                <p role="alert" className="inline-error">
                    {invalid}
                </p>
            ) : result.isPending ? (
                <Loading />
            ) : result.isError ? (
                <Failure
                    error={result.error}
                    reset={() => {
                        if (
                            result.error instanceof ApiFailure &&
                            result.error.status === 410 &&
                            paging.page > 0
                        )
                            paging.reset();
                        else void result.refetch();
                    }}
                />
            ) : (
                <>
                    <section className="panel flush">
                        <div className="table-scroll">
                            <table>
                                <thead>
                                    <tr>
                                        <th>开始时间</th>
                                        <th>请求 ID</th>
                                        <th>模型 / Provider</th>
                                        <th>状态</th>
                                        <th>结果</th>
                                        <th className="numeric">总词元</th>
                                        <th className="numeric">耗时</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {result.data.data.map((item) => (
                                        <tr key={item.requestId}>
                                            <td className="nowrap">{timeText(item.startedAt)}</td>
                                            <td>
                                                <Link
                                                    className="mono id-link"
                                                    to={`/requests/${encodeURIComponent(item.requestId)}`}
                                                    state={{
                                                        returnTo:
                                                            location.pathname + location.search,
                                                    }}
                                                >
                                                    {item.requestId}
                                                    <ArrowUpRight size={13} />
                                                </Link>
                                            </td>
                                            <td>
                                                <div>{item.model ?? "未知"}</div>
                                                <div className="muted small">
                                                    {item.provider ?? "未知"}
                                                </div>
                                            </td>
                                            <td>
                                                <span className="status-tag">
                                                    {label(item.state)}
                                                </span>
                                            </td>
                                            <td>{label(item.outcome)}</td>
                                            <td className="numeric">
                                                {numberText(item.totalTokens)}
                                            </td>
                                            <td className="numeric nowrap">
                                                {item.requestMs === null
                                                    ? "未知"
                                                    : `${numberText(item.requestMs)} ms`}
                                            </td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                        {!result.data.data.length && <Empty />}
                    </section>
                    <Pager
                        page={paging.page}
                        next={!!result.data.nextCursor && paging.page < 99}
                        previous={paging.previous}
                        advance={() => paging.next(result.data.nextCursor)}
                        busy={result.isFetching}
                    />
                    {paging.page >= 99 && (
                        <p className="muted">已达到本次浏览页数上限，请缩小筛选范围。</p>
                    )}
                </>
            )}
        </>
    );
}

export function Facts({ entries }: { entries: [string, ReactNode][] }) {
    return (
        <dl className="facts">
            {entries.map(([key, value]) => (
                <div key={key}>
                    <dt>{key}</dt>
                    <dd>{value ?? "未知"}</dd>
                </div>
            ))}
        </dl>
    );
}
const ms = (value: number | null) => (value === null ? "未知" : `${numberText(value)} ms`);
export function RequestDetail() {
    const { id = "" } = useParams();
    const location = useLocation();
    const returnTo =
        typeof location.state?.returnTo === "string" &&
        location.state.returnTo.startsWith("/requests?")
            ? location.state.returnTo
            : "/requests";
    const epoch = useSession((state) => state.epoch);
    const result = useQuery({
        queryKey: [epoch, "request", id],
        queryFn: ({ signal }) => api.request(id, signal),
    });
    const item = result.data?.data;
    return (
        <>
            <Link className="back-link" to={returnTo}>
                <ArrowLeft size={15} />
                请求记录
            </Link>
            <PageTitle
                action={<Refresh busy={result.isFetching} onClick={() => void result.refetch()} />}
            >
                请求详情
            </PageTitle>
            {result.isPending ? (
                <Loading />
            ) : result.isError ? (
                <Failure error={result.error} reset={() => void result.refetch()} />
            ) : (
                item && (
                    <>
                        <div className="detail-heading">
                            <code>{item.requestId}</code>
                            <span className="status-tag">{label(item.state)}</span>
                            <span>{label(item.outcome)}</span>
                            <Link to={`/logs?requestId=${encodeURIComponent(item.requestId)}`}>
                                查看关联日志 <ArrowUpRight size={14} />
                            </Link>
                        </div>
                        <section className="panel">
                            <h2>请求</h2>
                            <Facts
                                entries={[
                                    ["运行 ID", <code>{item.runId}</code>],
                                    ["开始时间", timeText(item.startedAt)],
                                    ["结束时间", timeText(item.finishedAt)],
                                    ["客户端流式", label(item.clientStream)],
                                    ["模型", item.model],
                                    ["Provider", item.provider],
                                    ["配置上游模型", item.configuredUpstreamModel],
                                    ["最终上游模型", item.finalUpstreamModel],
                                    ["上游状态码", item.upstreamStatus],
                                    ["上游媒体类型", item.upstreamMedia],
                                    ["上游完成", label(item.upstreamCompleted)],
                                ]}
                            />
                        </section>
                        <section className="panel">
                            <h2>
                                耗时{" "}
                                <HelpMark>
                                    来自请求记录的已知计时。缺失值保留为未知，各项不保证可相加。
                                </HelpMark>
                            </h2>
                            <Facts
                                entries={[
                                    ["请求耗时", ms(item.requestMs)],
                                    ["上游响应头", ms(item.upstreamHeadersMs)],
                                    ["首次输出", ms(item.firstOutputMs)],
                                ]}
                            />
                        </section>
                        <section className="panel">
                            <h2>
                                用量{" "}
                                <HelpMark>
                                    原生上游用量在脚本改写前提取。输入、输出、总计及细分项独立记录，不推断缺失计数。
                                </HelpMark>
                            </h2>
                            <Facts
                                entries={[
                                    ["输入词元", numberText(item.inputTokens)],
                                    ["输出词元", numberText(item.outputTokens)],
                                    ["总词元", numberText(item.totalTokens)],
                                    ["缓存输入词元", numberText(item.cachedInputTokens)],
                                    ["推理输出词元", numberText(item.reasoningOutputTokens)],
                                    ["用量来源", item.usageSource],
                                    ["用量质量", label(item.usageQuality)],
                                    ["客户端输入字节", numberText(item.inputBytes)],
                                    ["发往上游字节", numberText(item.outboundBytes)],
                                    ["上游响应字节", numberText(item.upstreamBytes)],
                                    ["客户端输出字节", numberText(item.outputBytes)],
                                ]}
                            />
                        </section>
                        <section className="panel">
                            <h2>错误</h2>
                            <Facts
                                entries={[
                                    ["错误码", item.errorCode ?? "无"],
                                    ["阶段", item.errorPhase ?? "无"],
                                    ["步骤", item.errorStep ?? "无"],
                                ]}
                            />
                        </section>
                    </>
                )
            )}
        </>
    );
}
