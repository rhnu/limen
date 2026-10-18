import { Link, useLocation, useParams, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, ArrowUpRight } from "lucide-react";
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
import { queries } from "../api/queries";
import { Filters } from "../components/filters";
import { useCursor } from "../hooks/use-cursor";
import { paths, requestPath, safeReturnTo } from "../routing/paths";
import {
    parseSearch,
    parseRequestId,
    requestsSearchSchema,
    emptySearchSchema,
    serializeSearch,
    recordStates,
    outcomes,
} from "../routing/search";
import { RequestPerformance } from "../components/performance";
import { downstreamModes, tpsText, ttftText } from "../lib/metrics";
import { Facts } from "../components/facts";

export function Requests() {
    const [params] = useSearchParams();
    const location = useLocation();
    const epoch = useSession((state) => state.epoch);
    const search = parseSearch(requestsSearchSchema, params);
    const paging = useCursor(search.canonical);
    const invalid = search.error;
    const state = search.filters?.state;
    const outcome = search.filters?.outcome;
    const result = useQuery(queries.requests(epoch, search.filters, paging.cursor));
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
                kind="requests"
                extra={
                    <>
                        <Field label="状态">
                            <select name="state" defaultValue={state ?? ""}>
                                <option value="">全部状态</option>
                                {recordStates.map((value) => (
                                    <option key={value} value={value}>
                                        {label(value)}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="结果">
                            <select name="outcome" defaultValue={outcome ?? ""}>
                                <option value="">全部结果</option>
                                {outcomes.map((value) => (
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
                                        <th className="numeric">
                                            下游 TTFT{" "}
                                            <HelpMark>
                                                请求开始至网关交付首生成内容，不代表客户端网络接收。缓冲合成路径单独标明。
                                            </HelpMark>
                                        </th>
                                        <th className="numeric">
                                            平均 TPS{" "}
                                            <HelpMark>
                                                原生输出词元 /
                                                上游总耗时；要求上游完整结束且用量可用。真实零值保留为零。
                                            </HelpMark>
                                        </th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {result.data.data.map((item) => (
                                        <tr key={item.requestId}>
                                            <td className="nowrap">{timeText(item.startedAt)}</td>
                                            <td>
                                                <Link
                                                    className="mono id-link"
                                                    to={requestPath(item.requestId)}
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
                                            <td className="numeric">
                                                {ttftText(item.metrics, "downstream")}
                                                <div className="muted small">
                                                    {downstreamModes[item.metrics.downstreamMode]}
                                                </div>
                                            </td>
                                            <td className="numeric">
                                                {tpsText(
                                                    item.metrics.averageOutputTps,
                                                    item.metrics.averageOutputTpsStatus,
                                                )}
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

export function RequestDetail() {
    const { id = "" } = useParams();
    const location = useLocation();
    const navigationState: unknown = location.state;
    const returnTo = safeReturnTo(
        navigationState !== null &&
            typeof navigationState === "object" &&
            "returnTo" in navigationState
            ? navigationState.returnTo
            : undefined,
        paths.requests,
    );
    const [params] = useSearchParams();
    const search = parseSearch(emptySearchSchema, params);
    const epoch = useSession((state) => state.epoch);
    const requestId = parseRequestId(id);
    const invalid = search.error ?? requestId.error;
    const result = useQuery(queries.request(epoch, requestId.id, !invalid));
    const item = result.data?.data;
    return (
        <>
            <Link className="back-link" to={returnTo}>
                <ArrowLeft size={15} />
                请求记录
            </Link>
            <PageTitle
                action={
                    <Refresh
                        disabled={!!invalid}
                        busy={result.isFetching}
                        onClick={() => void result.refetch()}
                    />
                }
            >
                请求详情
            </PageTitle>
            {invalid ? (
                <p role="alert" className="inline-error">
                    {invalid}
                </p>
            ) : result.isPending ? (
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
                            <Link
                                to={`${paths.logs}?${serializeSearch({ requestId: item.requestId })}`}
                            >
                                查看关联日志 <ArrowUpRight size={14} />
                            </Link>
                        </div>
                        <section className="panel">
                            <h2>请求</h2>
                            <Facts
                                entries={[
                                    ["运行 ID", <code key="run-id">{item.runId}</code>],
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
                        <RequestPerformance item={item} />
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
