import { useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { ApiFailure } from "../api/errors";
import { useSession } from "../session/store";
import { label, numberText } from "../lib/values";
import { PageTitle, Refresh, Loading, Failure, Empty, Pager } from "../components/common";
import { Facts } from "../components/facts";
import { queries } from "../api/queries";
import { useCursor } from "../hooks/use-cursor";
import { configSearchSchema, parseSearch, serializeSearch } from "../routing/search";
import type { ResourceLimits, ServerConfig } from "../api/generated/types.gen";

const limitLabels: Record<keyof ResourceLimits, string> = {
    startupMs: "启动期限 (ms)",
    hookMs: "Hook 期限 (ms)",
    requestMs: "请求期限 (ms)",
    executionMs: "同步执行预算 (ms)",
    vmWorkMs: "VM 工作预算 (ms)",
    cleanupMs: "清理预算 (ms)",
    shutdownMs: "关闭期限 (ms)",
    memoryBytes: "VM 内存 (B)",
    stackBytes: "VM 栈 (B)",
    concurrency: "生成并发",
    queueCapacity: "命令队列容量",
    maxPending: "最大待定 Promise",
    jobBatch: "微任务批次",
    maxBodyBytes: "请求正文上限 (B)",
    maxEventBytes: "事件上限 (B)",
    maxOutputEvents: "输出事件上限",
    maxObservationsPerHook: "每 Hook 观测上限",
    maxObservationsPerRequest: "每请求观测上限",
    maxOutputBytes: "输出上限 (B)",
    maxAggregateBytes: "聚合上限 (B)",
    maxAggregateEvents: "聚合事件上限",
    maxModules: "模块数量上限",
    maxSourceBytes: "模块源码上限 (B)",
    maxJsonDepth: "JSON 深度上限",
    maxJsonNodes: "JSON 节点上限",
};
const limitNames = new Map(Object.entries(limitLabels));

function Server({ config }: { config: ServerConfig }) {
    return (
        <>
            <section className="panel">
                <h2>服务</h2>
                <Facts
                    entries={[
                        ["监听地址", config.listen],
                        ["客户端密钥", config.apiKeyConfigured ? "已配置" : "未配置"],
                        ["管理密钥", config.managerKeyConfigured ? "已配置" : "未配置"],
                    ]}
                />
            </section>
            <section className="panel">
                <h2>存储</h2>
                <Facts
                    entries={[
                        ["保留期限", `${config.storage.retentionDays} 天`],
                        ["记录上限", numberText(config.storage.maxRecords)],
                        ["数据库上限", `${numberText(config.storage.maxDbBytes)} B`],
                        ["操作期限", `${config.storage.operationMs} ms`],
                    ]}
                />
            </section>
            <section className="panel">
                <h2>日志</h2>
                <Facts
                    entries={[
                        ["级别", label(config.log.level)],
                        ["标准错误格式", config.log.format === "text" ? "文本" : "JSON"],
                        ["文件大小上限", `${numberText(config.log.maxFileBytes)} B`],
                        ["备份数量", numberText(config.log.maxFiles)],
                    ]}
                />
            </section>
            <section className="panel">
                <h2>管理接口</h2>
                <Facts
                    entries={[
                        ["并发数", numberText(config.management.concurrency)],
                        ["总期限", `${config.management.timeoutMs} ms`],
                    ]}
                />
            </section>
            <section className="panel">
                <h2>资源限制</h2>
                <Facts
                    entries={Object.entries(config.limits).map(([key, value]) => [
                        limitNames.get(key) ?? key,
                        numberText(value),
                    ])}
                />
            </section>
        </>
    );
}
export function Config() {
    const [params, setParams] = useSearchParams();
    const search = parseSearch(configSearchSchema, params);
    const section = search.filters?.section ?? "server";
    const epoch = useSession((state) => state.epoch);
    const paging = useCursor(search.canonical);
    const result = useQuery(queries.config(epoch, search.filters, paging.cursor));
    const response = result.data;
    return (
        <>
            <PageTitle
                help="显示启动时加载的非秘密配置。密钥、路径、脚本、请求头与 defaults 不会返回。配置修改需要在服务端进行并重启。"
                action={
                    <Refresh
                        disabled={!!search.error}
                        busy={result.isFetching}
                        onClick={() => void result.refetch()}
                    />
                }
            >
                配置
            </PageTitle>
            <div className="tabs" role="group" aria-label="配置分区">
                {(
                    [
                        ["server", "服务"],
                        ["models", "模型"],
                        ["providers", "Provider"],
                    ] as const
                ).map(([key, name]) => (
                    <button
                        key={key}
                        type="button"
                        aria-pressed={section === key}
                        className={section === key ? "active" : ""}
                        onClick={() => setParams(serializeSearch({ section: key }))}
                    >
                        {name}
                    </button>
                ))}
            </div>
            {search.error ? (
                <p role="alert" className="inline-error">
                    {search.error}
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
                response && (
                    <>
                        {response.section === "server" ? (
                            <Server config={response.data} />
                        ) : (
                            <>
                                <section className="panel flush">
                                    <div className="table-scroll">
                                        <table>
                                            <thead>
                                                <tr>
                                                    {section === "models" ? (
                                                        <>
                                                            <th>模型别名</th>
                                                            <th>Provider</th>
                                                            <th>配置上游模型</th>
                                                        </>
                                                    ) : (
                                                        <>
                                                            <th>Provider</th>
                                                            <th>上游 Origin</th>
                                                        </>
                                                    )}
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {response.section === "models"
                                                    ? response.data.map((item) => (
                                                          <tr key={item.alias}>
                                                              <td>{item.alias}</td>
                                                              <td>{item.provider}</td>
                                                              <td>
                                                                  {item.configuredUpstreamModel}
                                                              </td>
                                                          </tr>
                                                      ))
                                                    : response.data.map((item) => (
                                                          <tr key={item.id}>
                                                              <td>{item.id}</td>
                                                              <td>{item.origin}</td>
                                                          </tr>
                                                      ))}
                                            </tbody>
                                        </table>
                                    </div>
                                    {!response.data.length && <Empty>此分区没有配置项。</Empty>}
                                </section>
                                <Pager
                                    page={paging.page}
                                    next={!!response.nextCursor && paging.page < 99}
                                    previous={paging.previous}
                                    advance={() => paging.next(response.nextCursor)}
                                    busy={result.isFetching}
                                />
                                {paging.page >= 99 && (
                                    <p className="muted">已达到本次浏览页数上限。</p>
                                )}
                            </>
                        )}
                    </>
                )
            )}
        </>
    );
}
