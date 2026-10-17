import { useState } from "react";
import { Link, useNavigate } from "react-router";
import { Login } from "../components/login";
import { useSession } from "../session/store";
import { paths } from "../routing/paths";

export function Home() {
    const [login, setLogin] = useState(false);
    const authenticated = useSession((state) => state.phase === "authenticated");
    const navigate = useNavigate();
    return (
        <div className="encyclopedia">
            <header className="article-masthead">
                <Link to={paths.home} className="wordmark">
                    LIMEN
                </Link>
            </header>
            <main className="article-layout">
                <nav className="article-toc" aria-label="文章目录">
                    <strong>目录</strong>
                    {(
                        [
                            ["overview", "概述"],
                            ["training", "训练与推理"],
                            ["tokens", "词元与上下文"],
                            ["conversation", "对话接口"],
                            ["limits", "能力与局限"],
                            ["gateway", "模型网关"],
                        ] as const
                    ).map(([id, title]) => (
                        <a
                            key={id}
                            href={`#/#${id}`}
                            onClick={(event) => {
                                event.preventDefault();
                                document.getElementById(id)?.scrollIntoView();
                            }}
                        >
                            {title}
                        </a>
                    ))}
                </nav>
                <article>
                    <h1 id="overview">大型语言模型</h1>
                    <p className="article-intro">
                        大型语言模型（Large Language
                        Model，LLM）是一类从大量文本及其他数据中学习语言统计规律的机器学习模型。模型根据已有上下文预测后续词元，通过反复生成词元，形成文本、代码或结构化结果。
                    </p>
                    <p>
                        语言模型可以完成问答、摘要、翻译和代码辅助等任务。它输出的是根据训练、上下文和采样规则产生的结果，表达流畅并不保证内容正确。实际系统通常将模型与检索、工具调用及人工判断结合使用。
                    </p>
                    <h2 id="training">训练与推理</h2>
                    <p>
                        训练通过调整模型参数，使模型在给定上下文时更准确地预测数据中的后续内容。预训练建立广泛的语言与知识模式；后续训练可使模型更适合遵循指令、展开对话或完成特定领域任务。
                    </p>
                    <p>
                        推理是使用已训练模型生成结果的过程。一次请求通常包含用户输入、系统指令以及部分对话历史。模型逐步计算候选词元的分布，再按解码策略选择输出。温度等采样参数影响结果的随机程度，但不能单独决定事实准确性。
                    </p>
                    <h2 id="tokens">词元与上下文</h2>
                    <p>
                        词元（token）是模型处理文本的基本单位，可能对应一个字、词的一部分、标点或其他片段。分词方式因模型而异，字符数不能精确换算为词元数。
                    </p>
                    <p>
                        上下文窗口限制一次推理可处理的输入和输出长度。对话历史、检索资料、工具描述和生成内容都可能占用这一容量。服务端报告的输入、输出及缓存词元是不同统计维度；部分服务或响应不会提供完整用量。
                    </p>
                    <h2 id="conversation">对话接口</h2>
                    <p>
                        对话接口通常以消息序列表示输入，每条消息带有角色和内容。请求中的模型标识选择目标模型，参数控制生成上限及采样行为。不同服务对消息内容、工具调用和多模态数据的支持并不完全相同。
                    </p>
                    <p>
                        非流式响应在处理完成后返回结果；流式响应则逐段发送生成事件。收到部分文本不代表请求成功完成，连接结束也不等同于模型协议正常终止。应用需要分别处理生成内容、结束信号、错误和用量报告。
                    </p>
                    <h2 id="limits">能力与局限</h2>
                    <p>
                        模型可能生成缺乏依据的事实、引用或推理步骤，也可能对相似输入给出不同答案。训练数据中的偏差与知识时效会影响输出。长上下文并不保证每一处信息都会被同等准确地利用。
                    </p>
                    <p>
                        涉及重要决策时，应核对原始资料并保留人的判断。输入给模型或工具的数据还受到相应服务的数据处理方式、权限和保存规则约束。工具调用应由应用验证参数与权限，而非直接信任生成文本。
                    </p>
                    <h2 id="gateway">模型网关</h2>
                    <p>
                        模型网关位于客户端与模型服务之间，负责认证、模型别名、请求路由和协议适配。统一接口能够降低应用接入多个服务的成本，但上游能力与语义差异仍需要明确处理。
                    </p>
                    <p>
                        Limen 是面向个人与私有网络的 Chat Completions 网关。它使用显式模型别名和本地
                        JavaScript
                        管线连接上游服务，并记录有限的请求元数据与用量。客户端凭据与上游凭据分别管理，运行日志不保存对话正文。
                    </p>
                </article>
            </main>
            <footer className="article-colophon">
                <span>Limen</span>
                <button
                    type="button"
                    className="colophon-mark"
                    aria-label="打开管理入口"
                    title="管理入口"
                    onClick={() => {
                        if (authenticated) void navigate(paths.usage);
                        else setLogin(true);
                    }}
                >
                    ※
                </button>
            </footer>
            <Login open={login} onOpenChange={setLogin} />
        </div>
    );
}
