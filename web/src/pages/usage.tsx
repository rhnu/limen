import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import {
    ResponsiveContainer,
    BarChart,
    Bar,
    XAxis,
    YAxis,
    CartesianGrid,
    Tooltip,
    Legend,
} from "recharts";
import { queries } from "../api/queries";
import { parseSearch, usageSearchSchema } from "../routing/search";
import { Filters } from "../components/filters";
import { useSession } from "../session/store";
import { label, numberText, safeChartValue, timeText, usageTotals } from "../lib/values";
import { PageTitle, Refresh, Field, Loading, Failure, Empty, HelpMark } from "../components/common";
import type { UsageGroup } from "../api/generated/types.gen";

function Quality({ group }: { group: UsageGroup }) {
    return (
        <details>
            <summary>统计详情</summary>
            <div className="quality-grid">
                {[
                    { title: "状态", values: group.states },
                    { title: "结果", values: group.outcomes },
                    { title: "用量质量", values: group.qualities },
                    { title: "上游完成", values: group.upstreamCompleted },
                ].map(({ title, values }) => (
                    <div key={title}>
                        <strong>{title}</strong>
                        {Object.entries(values).map(([key, value]) => (
                            <span key={key}>
                                {label(key)}：{numberText(value)}
                            </span>
                        ))}
                    </div>
                ))}
                <div>
                    <strong>独立用量样本</strong>
                    {[
                        { title: "输入", token: group.tokens.input },
                        { title: "输出", token: group.tokens.output },
                        { title: "总计", token: group.tokens.total },
                        { title: "缓存输入", token: group.tokens.cachedInput },
                        { title: "推理输出", token: group.tokens.reasoningOutput },
                    ].map(({ title, token }) => (
                        <span key={title}>
                            {title}：{numberText(token.sum)} / {token.samples} 个样本
                        </span>
                    ))}
                </div>
            </div>
        </details>
    );
}

export function Usage() {
    const [params] = useSearchParams();
    const epoch = useSession((state) => state.epoch);
    const search = parseSearch(usageSearchSchema, params);
    const groupBy = search.filters?.groupBy ?? "day";
    const invalid = search.error;
    const result = useQuery(queries.usage(epoch, search.filters));
    const summary = result.data?.data;
    const totals = summary ? usageTotals(summary.groups) : null;
    const chart = summary?.groups.map((item) => ({
        name: item.key ?? (groupBy === "none" ? "全部" : "未知"),
        input: safeChartValue(item.tokens.input.sum),
        output: safeChartValue(item.tokens.output.sum),
    }));
    const chartOverflow = summary?.groups.some((item) =>
        [item.tokens.input.sum, item.tokens.output.sum].some(
            (value) => value !== null && safeChartValue(value) === null,
        ),
    );
    const hasChartValues = chart?.some((item) => item.input !== null || item.output !== null);
    return (
        <>
            <PageTitle
                help="汇总包含失败和取消请求的已知消耗。各项用量独立计数；未知值不会作为零。按日分组使用 UTC。"
                action={
                    <Refresh
                        busy={result.isFetching}
                        disabled={!!invalid}
                        onClick={() => void result.refetch()}
                    />
                }
            >
                用量
            </PageTitle>
            <Filters
                kind="usage"
                extra={
                    <Field label="分组">
                        <select name="groupBy" defaultValue={groupBy}>
                            <option value="day">按日</option>
                            <option value="model">按模型</option>
                            <option value="provider">按 Provider</option>
                            <option value="none">不分组</option>
                        </select>
                    </Field>
                }
            />
            {invalid ? (
                <p role="alert" className="inline-error">
                    {invalid}
                </p>
            ) : result.isPending ? (
                <Loading />
            ) : result.isError ? (
                <Failure error={result.error} reset={() => void result.refetch()} />
            ) : (
                summary &&
                totals && (
                    <>
                        <div className="stats">
                            <div>
                                <span>请求数</span>
                                <strong>{numberText(totals.records)}</strong>
                            </div>
                            {[
                                { title: "输入词元", value: totals.input },
                                { title: "输出词元", value: totals.output },
                                { title: "总词元", value: totals.total },
                            ].map(({ title, value }) => {
                                return (
                                    <div key={title}>
                                        <span>
                                            {title}{" "}
                                            <HelpMark>
                                                {value.samples}{" "}
                                                个已知样本。各项独立汇总，输入与输出之和不一定等于报告的总数。
                                            </HelpMark>
                                        </span>
                                        <strong>{numberText(value.sum)}</strong>
                                    </div>
                                );
                            })}
                        </div>
                        <div className="range-line">
                            <span>
                                {timeText(summary.from)} — {timeText(summary.to)}
                            </span>
                            <HelpMark>
                                显示为 UTC 时间。保留期限 {summary.retention.retentionDays}{" "}
                                天；记录上限 {numberText(summary.retention.maxRecords)}
                                。最早保留记录：{timeText(summary.retention.oldestStartedAt)}
                                。保留边界不保证此前历史完整。
                            </HelpMark>
                        </div>
                        {!summary.groups.length ? (
                            <Empty>此时间范围没有用量记录。</Empty>
                        ) : (
                            <>
                                {chartOverflow ? (
                                    <div className="notice">
                                        部分计数超过图表的精确表示范围，请查看下表。
                                    </div>
                                ) : !hasChartValues ? (
                                    <div className="notice">此范围没有已知的输入或输出词元值。</div>
                                ) : (
                                    <section
                                        className="panel chart-panel"
                                        aria-label="已知词元用量图"
                                    >
                                        <h2>已知词元用量</h2>
                                        <ResponsiveContainer width="100%" height={270}>
                                            <BarChart
                                                data={chart}
                                                margin={{ top: 20, right: 10, bottom: 0, left: 10 }}
                                                accessibilityLayer
                                            >
                                                <CartesianGrid
                                                    strokeDasharray="0"
                                                    vertical={false}
                                                    stroke="#e8e8e8"
                                                />
                                                <XAxis
                                                    dataKey="name"
                                                    axisLine={false}
                                                    tickLine={false}
                                                    fontSize={12}
                                                />
                                                <YAxis
                                                    axisLine={false}
                                                    tickLine={false}
                                                    fontSize={12}
                                                />
                                                <Tooltip
                                                    formatter={(value) =>
                                                        numberText(
                                                            typeof value === "number"
                                                                ? value
                                                                : undefined,
                                                        )
                                                    }
                                                />
                                                <Legend />
                                                <Bar
                                                    dataKey="input"
                                                    name="输入词元"
                                                    fill="#464646"
                                                    radius={[2, 2, 0, 0]}
                                                />
                                                <Bar
                                                    dataKey="output"
                                                    name="输出词元"
                                                    fill="#bfc0c3"
                                                    radius={[2, 2, 0, 0]}
                                                />
                                            </BarChart>
                                        </ResponsiveContainer>
                                    </section>
                                )}
                                <section className="panel">
                                    <h2>
                                        用量汇总{" "}
                                        <HelpMark>
                                            表内只展示已知值。展开统计详情可查看样本数、记录状态、结果和用量质量。
                                        </HelpMark>
                                    </h2>
                                    <div className="table-scroll">
                                        <table>
                                            <thead>
                                                <tr>
                                                    <th>
                                                        {groupBy === "none"
                                                            ? "范围"
                                                            : label(groupBy)}
                                                    </th>
                                                    <th className="numeric">请求数</th>
                                                    <th className="numeric">输入词元</th>
                                                    <th className="numeric">输出词元</th>
                                                    <th className="numeric">总词元</th>
                                                    <th>样本与质量</th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {summary.groups.map((item, index) => (
                                                    <tr key={item.key ?? index}>
                                                        <td>
                                                            {item.key ??
                                                                (groupBy === "none"
                                                                    ? "全部"
                                                                    : "未知")}
                                                        </td>
                                                        <td className="numeric">
                                                            {numberText(item.records)}
                                                        </td>
                                                        <td className="numeric">
                                                            {numberText(item.tokens.input.sum)}
                                                        </td>
                                                        <td className="numeric">
                                                            {numberText(item.tokens.output.sum)}
                                                        </td>
                                                        <td className="numeric">
                                                            {numberText(item.tokens.total.sum)}
                                                        </td>
                                                        <td>
                                                            <Quality group={item} />
                                                        </td>
                                                    </tr>
                                                ))}
                                            </tbody>
                                        </table>
                                    </div>
                                </section>
                            </>
                        )}
                    </>
                )
            )}
        </>
    );
}
