import { useQuery } from "@tanstack/react-query";
import { queries } from "../api/queries";
import { useSearchParams } from "react-router";
import { parseSearch, emptySearchSchema } from "../routing/search";
import { useSession } from "../session/store";
import { label, numberText, timeText } from "../lib/values";
import { PageTitle, Refresh, Loading, Failure, HelpMark } from "../components/common";
import { Facts } from "../components/facts";

export function Status() {
    const epoch = useSession((state) => state.epoch);
    const [params] = useSearchParams();
    const search = parseSearch(emptySearchSchema, params);
    const result = useQuery(queries.status(epoch, !search.error));
    const status = result.data?.data;
    return (
        <>
            <PageTitle
                help="HTTP 200 仅表示状态可读取。是否接收新的生成请求由实际接入状态决定。"
                action={
                    <Refresh
                        disabled={!!search.error}
                        busy={result.isFetching}
                        onClick={() => void result.refetch()}
                    />
                }
            >
                系统状态
            </PageTitle>
            {search.error ? (
                <p role="alert" className="inline-error">
                    {search.error}
                </p>
            ) : result.isPending ? (
                <Loading />
            ) : result.isError ? (
                <Failure error={result.error} reset={() => void result.refetch()} />
            ) : (
                status && (
                    <>
                        <div
                            className={`admission ${status.acceptingGenerations ? "" : "unavailable"}`}
                        >
                            <span className="status-dot" />
                            <strong>
                                {status.acceptingGenerations
                                    ? "正在接收生成请求"
                                    : "未接收生成请求"}
                            </strong>
                        </div>
                        <section className="panel">
                            <h2>进程</h2>
                            <Facts
                                entries={[
                                    ["版本", status.version],
                                    ["运行 ID", <code key="run-id">{status.runId}</code>],
                                    ["启动时间", timeText(status.startedAt)],
                                    ["VM", label(status.vm)],
                                    ["存储", label(status.storage.state)],
                                    ["存储原因", status.storage.reason ?? "无"],
                                ]}
                            />
                        </section>
                        <section className="panel">
                            <h2>
                                日志输出{" "}
                                <HelpMark>
                                    日志队列或写入故障不会阻止生成。丢弃与 I/O
                                    错误分别计数，运行日志不作为用量数据来源。
                                </HelpMark>
                            </h2>
                            <div className="sink-grid">
                                {(["file", "stderr"] as const).map((sink) => (
                                    <div key={sink}>
                                        <h3>{sink === "file" ? "滚动文件" : "标准错误"}</h3>
                                        <Facts
                                            entries={[
                                                ["状态", label(status.logging[sink].state)],
                                                [
                                                    "丢弃记录",
                                                    numberText(status.logging[sink].dropped),
                                                ],
                                                [
                                                    "I/O 错误",
                                                    numberText(status.logging[sink].ioErrors),
                                                ],
                                                [
                                                    "最近错误",
                                                    status.logging[sink].lastError ?? "无",
                                                ],
                                            ]}
                                        />
                                    </div>
                                ))}
                            </div>
                        </section>
                    </>
                )
            )}
        </>
    );
}
