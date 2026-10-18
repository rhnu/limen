import type {
    DurationAggregate,
    PerformanceSummary,
    PhaseAggregate,
    PhaseTiming,
    RequestDetail,
} from "../api/generated/types.gen";
import {
    decimalText,
    downstreamModes,
    microsecondsText,
    parameterRows,
    timingFields,
    tpsText,
    ttftText,
    transportText,
    firstTextText,
    finishReasonText,
} from "../lib/metrics";
import { label, numberText } from "../lib/values";
import { Facts } from "./facts";
import { HelpMark } from "./common";

const performanceFields: {
    key: Exclude<keyof PerformanceSummary, "records">;
    title: string;
    unit: string;
}[] = [
    { key: "requestMs", title: "请求耗时", unit: "ms" },
    { key: "upstreamTtftMs", title: "原生上游 TTFT", unit: "ms" },
    { key: "downstreamTtftMs", title: "原生下游交付 TTFT", unit: "ms" },
    { key: "upstreamFirstTextMs", title: "上游首正文文本", unit: "ms" },
    { key: "downstreamFirstTextMs", title: "下游首正文文本", unit: "ms" },
    { key: "averageOutputTps", title: "平均输出 TPS", unit: "token/s" },
    { key: "generationTps", title: "生成 TPS 估算", unit: "token/s" },
];
export function PerformanceTable({ performance }: { performance: PerformanceSummary }) {
    return (
        <div className="table-scroll">
            <table>
                <thead>
                    <tr>
                        <th>指标</th>
                        <th className="numeric">样本</th>
                        {["平均", "最小", "最大", "P50", "P95", "P99"].map((title) => (
                            <th key={title} className="numeric">
                                {title}
                            </th>
                        ))}
                    </tr>
                </thead>
                <tbody>
                    {performanceFields.map(({ key, title, unit }) => (
                        <tr key={key}>
                            <td>
                                {title}
                                <span className="muted"> ({unit})</span>
                            </td>
                            <td className="numeric">{numberText(performance[key].samples)}</td>
                            {(["mean", "min", "max", "p50", "p95", "p99"] as const).map((stat) => (
                                <td className="numeric" key={stat}>
                                    {decimalText(performance[key][stat])}
                                </td>
                            ))}
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}
const phaseLabel = (phase: string) =>
    ({
        onRequest: "请求",
        beforeSend: "发送前",
        afterReceive: "收到响应",
        onResponse: "完整响应",
        onUpstreamEvent: "上游事件",
        onStreamEvent: "下游事件",
        onContent: "正文",
        onErrorResponse: "错误响应",
        onError: "错误观察",
        onFinish: "终结观察",
    })[phase] ?? phase;
const executorLabel = (executor: "script" | "native") =>
    executor === "script" ? "JavaScript" : "原生";
const phaseTitle = (item: PhaseTiming | PhaseAggregate) =>
    `${phaseLabel(item.phase)} · ${label(item.scope)} · ${executorLabel(item.executor)}`;
function CallCounts({ item }: { item: PhaseTiming | PhaseAggregate }) {
    return (
        <span>
            调用 {item.calls} · 报告 {item.reportedCalls} · 失败 {item.failedCalls} · 中断{" "}
            {item.interruptedCalls} · 预算跳过 {item.skippedBudgetCalls} · 不可用{" "}
            {item.unavailableCalls}
        </span>
    );
}
const phaseParts = [
    ["queue", "排队"],
    ["initialCall", "初次调用"],
    ["pending", "Promise 等待"],
    ["inputBridge", "输入桥接"],
    ["outputBridge", "输出桥接"],
] as const;
export function PhaseTimings({ phases }: { phases: PhaseTiming[] }) {
    if (!phases.length) return <p className="muted small">没有已记录的策略调用。</p>;
    return (
        <div className="table-scroll">
            <table>
                <thead>
                    <tr>
                        <th>阶段 / Scope / 执行器</th>
                        <th className="numeric">Wall</th>
                        {phaseParts.map(([key, title]) => (
                            <th className="numeric" key={key}>
                                {title} / 样本
                            </th>
                        ))}
                    </tr>
                </thead>
                <tbody>
                    {phases.map((item) => (
                        <tr key={`${item.phase}-${item.scope}-${item.executor}`}>
                            <td>
                                <div>{phaseTitle(item)}</div>
                                <div className="muted small">
                                    <CallCounts item={item} />
                                </div>
                            </td>
                            <td className="numeric">{microsecondsText(item.wallUs)}</td>
                            {phaseParts.map(([key]) => (
                                <td className="numeric" key={key}>
                                    {microsecondsText(item[key].totalUs)} / {item[key].samples}
                                </td>
                            ))}
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}
function Aggregate({ value }: { value: DurationAggregate }) {
    return (
        <span>
            {decimalText(value.sumMs)} / {decimalText(value.meanMs)} / {value.samples}
        </span>
    );
}
export function PhaseAggregates({ phases }: { phases: PhaseAggregate[] }) {
    if (!phases.length) return <p className="muted small">没有符合性能结果筛选的策略样本。</p>;
    return (
        <div className="table-scroll">
            <table>
                <thead>
                    <tr>
                        <th>阶段 / Scope / 执行器</th>
                        <th className="numeric">Wall</th>
                        {phaseParts.map(([key, title]) => (
                            <th className="numeric" key={key}>
                                {title}
                            </th>
                        ))}
                    </tr>
                </thead>
                <tbody>
                    {phases.map((item) => (
                        <tr key={`${item.phase}-${item.scope}-${item.executor}`}>
                            <td>
                                <div>{phaseTitle(item)}</div>
                                <div className="muted small">
                                    <CallCounts item={item} />
                                </div>
                            </td>
                            <td className="numeric">
                                <Aggregate value={item.wall} />
                            </td>
                            {phaseParts.map(([key]) => (
                                <td className="numeric" key={key}>
                                    <Aggregate value={item[key]} />
                                </td>
                            ))}
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}
export function StageAggregates({ stages }: { stages: Record<string, DurationAggregate> }) {
    return (
        <div className="table-scroll">
            <table>
                <thead>
                    <tr>
                        <th>阶段 / 里程碑</th>
                        <th className="numeric">累计 ms</th>
                        <th className="numeric">平均 ms</th>
                        <th className="numeric">样本</th>
                    </tr>
                </thead>
                <tbody>
                    {timingFields.map(({ key, title }) => {
                        const value = stages[key.slice(0, -2)];
                        return (
                            <tr key={key}>
                                <td>{title}</td>
                                <td className="numeric">{decimalText(value?.sumMs ?? null)}</td>
                                <td className="numeric">{decimalText(value?.meanMs ?? null)}</td>
                                <td className="numeric">{numberText(value?.samples ?? 0)}</td>
                            </tr>
                        );
                    })}
                </tbody>
            </table>
        </div>
    );
}
export function RequestPerformance({ item }: { item: RequestDetail }) {
    return (
        <>
            <section className="panel">
                <h2>
                    延迟与吞吐{" "}
                    <HelpMark>
                        上游 TTFT 从上游发送起算；下游 TTFT 从请求开始起算，以网关交付 body
                        为准，不代表客户端网络接收。首生成内容包含推理或工具增量，首正文文本单独记录。缓冲交付不能还原
                        token 时序。
                    </HelpMark>
                </h2>
                <Facts
                    entries={[
                        ["传输路径", transportText(item.clientStream, item.upstreamMedia)],
                        ["交付模式", downstreamModes[item.metrics.downstreamMode]],
                        ["原生上游 TTFT", ttftText(item.metrics, "upstream", item.upstreamMedia)],
                        ["下游交付 TTFT", ttftText(item.metrics, "downstream")],
                        [
                            "上游首正文文本",
                            firstTextText(
                                item.metrics.upstreamFirstTextMs,
                                item.upstreamMedia,
                                true,
                            ),
                        ],
                        [
                            "下游首正文文本",
                            firstTextText(item.metrics.downstreamFirstTextMs, item.upstreamMedia),
                        ],
                        [
                            "平均输出 TPS",
                            tpsText(
                                item.metrics.averageOutputTps,
                                item.metrics.averageOutputTpsStatus,
                            ),
                        ],
                        [
                            "生成 TPS 估算",
                            tpsText(item.metrics.generationTps, item.metrics.generationTpsStatus),
                        ],
                        ["已观察 choice", numberText(item.generation.choiceCount)],
                        ["独立生成观测", numberText(item.generation.upstreamContentObservations)],
                        ["观察到推理", label(item.generation.observedReasoning)],
                        ["观察到音频", label(item.generation.observedAudio)],
                        ["生成范围可估算", label(item.generation.contentQualified)],
                        ["结束原因分类", finishReasonText(item.generation, item.upstreamCompleted)],
                    ]}
                />
                <p className="muted small metric-note">
                    平均输出 TPS = 输出词元 / 上游总耗时。生成 TPS
                    仅对满足用量、完整性及独立时序条件的原生 SSE 估算。
                </p>
            </section>
            <section className="panel">
                <h2>
                    里程碑与阶段耗时{" "}
                    <HelpMark>
                        全部以单调时钟记录，微秒转为毫秒显示。里程碑与累计等待可重叠，不能相加。空值保留为未知。
                    </HelpMark>
                </h2>
                <div className="table-scroll">
                    <table>
                        <thead>
                            <tr>
                                <th>项目</th>
                                <th className="numeric">耗时 / 偏移</th>
                                <th>口径</th>
                            </tr>
                        </thead>
                        <tbody>
                            {timingFields.map(({ key, title, help }) => (
                                <tr key={key}>
                                    <td>{title}</td>
                                    <td className="numeric">
                                        {microsecondsText(item.timings[key])}
                                    </td>
                                    <td className="muted">{help}</td>
                                </tr>
                            ))}
                            {(
                                [
                                    [
                                        "上游首生成内容",
                                        item.generation.upstreamFirstContentUs,
                                        "自上游发送起算",
                                    ],
                                    [
                                        "上游末生成内容",
                                        item.generation.upstreamLastContentUs,
                                        "自上游发送起算",
                                    ],
                                    [
                                        "下游首生成内容",
                                        item.generation.downstreamFirstContentUs,
                                        "自请求开始起算；交付 body",
                                    ],
                                ] as const
                            ).map(([title, value, help]) => (
                                <tr key={title}>
                                    <td>{title}</td>
                                    <td className="numeric">{microsecondsText(value)}</td>
                                    <td className="muted">{help}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            </section>
            <section className="panel">
                <h2>
                    策略阶段{" "}
                    <HelpMark>
                        按阶段、Scope 与 JavaScript /
                        原生执行器分别统计。细分样本分母独立；未报告不能当作零。Promise 等待不是 CPU
                        时间，清理调用不回写生成阶段冻结统计。
                    </HelpMark>
                </h2>
                <PhaseTimings phases={item.phases} />
            </section>
            <section className="panel">
                <h2>
                    生成参数对照{" "}
                    <HelpMark>
                        仅展示已保存的固定白名单参数与安全摘要。原始参数在 defaults
                        与策略前采集，上游参数在最终编码成功后采集。未记录不代表默认值，未发送表示没有上游快照。
                    </HelpMark>
                </h2>
                {!parameterRows(item.parameters.client, item.parameters.upstream).length ? (
                    <p className="muted small">没有已记录的生成参数。</p>
                ) : (
                    <div className="table-scroll">
                        <table className="parameter-table">
                            <thead>
                                <tr>
                                    <th>参数</th>
                                    <th>客户端原始值</th>
                                    <th>发往上游</th>
                                </tr>
                            </thead>
                            <tbody>
                                {parameterRows(
                                    item.parameters.client,
                                    item.parameters.upstream,
                                ).map((row) => (
                                    <tr key={row.key}>
                                        <td>
                                            <code>{row.key}</code>
                                        </td>
                                        <td>
                                            <code>{row.client}</code>
                                        </td>
                                        <td>
                                            <code>{row.upstream}</code>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </section>
        </>
    );
}
