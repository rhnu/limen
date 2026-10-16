# Limen Chat Completions 与 SSE 兼容性调研

调研日期：2026-10-16。代码基线：`cd6c335c5d4b2baaf2e8ccdde43c5cd08222e8b5`。

结论：Limen 的请求侧已经比较开放，主要阻碍集中在响应侧。当前管线把 SSE 分帧、Chat chunk 校验、choice 完成状态、业务 hook 和聚合转换绑定起来，导致合法的协议扩展在适配前被拒绝。同形传递也承担了转换器级别的语义限制。值得重构这些边界，继续按供应商增加例外会延续问题。

本报告中的架构是建议，尚未成为项目契约。范围覆盖 Chat Completions 本体、OpenAI 兼容接口及其变体；Anthropic Messages、Gemini 原生接口只用于确定未来适配边界，不纳入本轮实现范围。Responses、UI、自动重试、故障转移和热重载均不属于建议的扩展范围。

## 当前管线实际支持什么

| 路径        | 已有能力                                                                       | 主要限制                                                                        |
| ----------- | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------- |
| 请求        | JSON 外壳校验；模型路由；任意扩展字段、消息内部结构和多模态输入保留            | `stream` 出现时必须是布尔值；每个 `beforeSend` 后仍要求 Chat 请求外壳           |
| JSON → JSON | 合法有界 JSON 对象；不要求完整 Chat response schema；保留扩展                  | 媒体类型只接受 `application/json`；hook 输出仍须对象                            |
| SSE → SSE   | 文本、refusal、function tools、多 choice、usage；允许未知 JSON 键随 chunk 传递 | 每个数据事件必须先通过严格 Chat chunk 和完成状态校验；丢弃 SSE 外层元数据与注释 |
| SSE → JSON  | 聚合文本、refusal、function tools、usage 和少量元数据                          | root、choice、delta、tool 各层白名单；非空 logprobs、推理、多模态等拒绝转换     |
| JSON → SSE  | 对支持的字段合成增量事件，字符串按 UTF-8 边界拆分                              | 同样受白名单限制；无法还原真实生成时序                                          |

依据：[请求校验](../src/gateway/logic.rs)第 74–132 行、[四路径选择](../src/gateway/upstream.rs)第 21–118 行、[转换根字段](../modules/chat/src/aggregate.rs)第 11–24 行、[JSON 流转换](../modules/chat/src/convert.rs)第 35–99 行。请求侧的宽松不应在此次重构中被一套更窄的完整请求类型替换。

## 官方协议与变体暴露的缺口

下表是文档和代码的交叉判断，不是对这些服务的真实联网认证。

| 协议或行为                                       | 官方证据                                                                       | Limen 当前表现与影响                                                                                               |
| ------------------------------------------------ | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| OpenAI 普通文本、工具、refusal、空 choices usage | 官方流 schema 包含这些结构                                                     | 常见路径已有支持。未知 finish reason 字符串也能保留，不限于 `stop`                                                 |
| OpenAI 音频尾更新                                | 最后一次 `delta.audio` 可只有 `expires_at`，该 choice 省略 `finish_reason`     | `validate_chunk` 强制该字段存在，SSE 同形也在 hook 前拒绝；仅把缺失补成 null 仍不足以解决完成后的音频元数据更新    |
| OpenAI `obfuscation`、moderation 等扩展          | 流 schema 默认包含 `obfuscation`，也定义 moderation                            | 同形一般能保留；`obfuscation` 已足以使 SSE → JSON 被根字段白名单拒绝。此字段属于传输属性，转换应有明确处置语义     |
| Azure 异步内容标注                               | prompt 标注可以使用空 `object`；choice 标注没有 `delta`；标注可晚于内容 finish | 首个标注即可在 hook 前失败；“choice 已 finish 就不能再有事件”的模型也表达不了后置标注                              |
| Mistral 推理流                                   | `delta.content` 在 thinking 阶段是内容块数组，答案阶段可以变回字符串           | `content` 只接受 string/null，SSE 同形在 hook 前失败；JSON 同形可传递数组                                          |
| DeepSeek、vLLM 推理                              | 分别存在 `reasoning_content`、`reasoning`；vLLM 文档说明过字段迁移             | 未完成 choice 中的这些扩展一般能同形传递；跨形转换因 delta 白名单失败                                              |
| vLLM 连续 usage                                  | `stream_options.continuous_usage_stats` 允许逐 chunk 发送累计用量              | 同形可输出，但原生统计把递增值判为冲突并清空计数；聚合把整个 usage 对象当固定元数据，第二次变化即失败              |
| OpenRouter 推理和统计                            | 包含结构化 `reasoning_details`、扩展元数据和统计尾事件                         | 指定统计尾形状已有兼容；推理细节、provider、native finish 等跨形仍被拒绝；继续增加尾事件白名单不能解决整体扩展问题 |
| Qwen OpenAI 兼容接口                             | 有视频、音频、推理等请求扩展                                                   | 请求内部对象基本能通过；响应和跨形能力必须按实际形状判断，不能据输入能通过就声称完整多模态兼容                     |
| Ollama OpenAI 兼容端点                           | 官方明确只支持 OpenAI API 的子集                                               | 应测试 `/v1/chat/completions`。原生 `/api/chat` 的 NDJSON 不应被自动当作 SSE；没有理由让本轮顺带扩展原生接口       |

对应一手资料：[OpenAI streaming schema](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events)、[Azure 内容流及标注示例](https://learn.microsoft.com/en-us/azure/foundry/openai/concepts/content-streaming)、[Mistral reasoning](https://docs.mistral.ai/capabilities/reasoning)、[DeepSeek Chat Completions](https://api-docs.deepseek.com/api/create-chat-completion/)、[vLLM reasoning](https://docs.vllm.ai/en/stable/features/reasoning_outputs/)、[vLLM StreamOptions 源码](https://github.com/vllm-project/vllm/blob/main/vllm/entrypoints/generate/base/protocol.py)、[vLLM 连续 usage 实现](https://docs.vllm.ai/en/v0.10.2/api/vllm/entrypoints/openai/serving_chat.html)、[OpenRouter reasoning](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)、[OpenRouter streaming](https://openrouter.ai/docs/api/reference/streaming)、[Qwen 接口](https://www.alibabacloud.com/help/en/model-studio/qwen-api-via-openai-chat-completions)、[Ollama 兼容范围](https://docs.ollama.com/api/openai-compatibility)。连续 usage 的当前开关与旧版本实现相互印证，不将旧版本默认值外推到当前版本。

### 已执行的最小合成实验

使用独立临时 Cargo 项目，通过 path 依赖调用当前 `limen-chat` 公共纯函数，`cargo run --offline`，没有连接供应商。

| 合成输入                                                | 实际结果                                                                                                      |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| 音频 expiry 更新省略 `finish_reason`                    | `validate_chunk` → `InvalidChunk`                                                                             |
| 合法 chunk 添加顶层 `obfuscation`                       | `validate_chunk` 成功；`Aggregator::push` → `Unsupported`；移除后该事件可加入聚合                             |
| `delta.content` 使用文本块数组                          | `validate_chunk` → `InvalidChunk`                                                                             |
| Azure 的空 `object`；另一个含标注、无 `delta` 的 choice | 两者分别触发 `InvalidChunk`                                                                                   |
| prompt tokens 固定，completion tokens 从 1 累计至 2     | `ChunkTracker` 正常结束；第二次聚合 → `ConflictingMetadata`；`UsageTracker` → `Conflicting`，有效计数全部为空 |

关键位置是 [chunk.rs](../modules/chat/src/chunk.rs)第 39–94 行、[aggregate.rs](../modules/chat/src/aggregate.rs)第 107–134、238–249 行和 [usage.rs](../modules/chat/src/usage.rs)第 61–115 行。现有 hook 可在聚合前规整扩展，但无法修复最早的 chunk 拒绝，而且原生 usage 在 hook 前已经采样。

## SSE 兼容需要区分的三个层次

### 分帧兼容

SSE 是 UTF-8 文本流，事件可以有 `event`、`id`、`retry` 和多行 `data`，允许注释、BOM、CR/LF/CRLF。命名事件本身是合法 SSE；它是否属于某个 Chat 方言，应由协议解释层判断。EOF 不会自动提交尚未以空行结束的事件。依据：[WHATWG SSE](https://html.spec.whatwg.org/multipage/server-sent-events.html)。

Limen 对 BOM、多行 data、常见换行已有测试。主要问题是 [Decoder](../modules/chat/src/sse.rs)第 70–89 行直接把 SSE 事件解释成 Chat：只接受默认 message，命名事件拒绝，`id/retry` 与注释无法流入后续链路。对标准 data-only Chat 这通常无影响，对更宽的方言和原样传递则是能力缺口。解析器不必执行 `retry` 的重连建议；保留字段也不代表启用自动重试。

### 存活与输出兼容

DeepSeek 明确会在等待推理时发送 SSE keep-alive 注释。Limen 能解析但不会转发它们，且 [Delivery::start](../src/gateway/response.rs)第 75–122 行到首个实际输出才发送响应。因此，上游仍活跃不等于下游能看到活动；经过有空闲超时的客户端或反向代理时，存在超时风险。这一影响是根据代码作出的推断，未在真实代理链路复现。依据：[DeepSeek keep-alive](https://api-docs.deepseek.com/quick_start/rate_limit)。

建议允许有界转发注释，或由统一输出器生成可配置心跳。发送响应头或心跳会提交 HTTP 状态，之后失败只能流内表达；这一取舍应成为明确行为。心跳应消耗字节与时间预算，但不能计为模型输出内容或重置总请求期限。JSON 下游仍应聚合，不能混入 SSE 注释。

### 完成与失败兼容

必须分别保存：HTTP 读取是否正常结束、是否观察到协议终止、每个 choice 的生成结束原因、是否仍有 usage/音频/标注尾事件、是否存在流内错误。`length`、`content_filter`、工具调用等是有效终止原因，不应被统一改成 `stop`；终止也不保证得到完整可用答案。DeepSeek 还有资源不足的原生结束原因，应保留其含义。[DeepSeek schema](https://api-docs.deepseek.com/api/create-chat-completion/)

目前“必须 DONE”对于采用该契约的 OpenAI 类流有依据，不建议全局放开为“EOF 就成功”。需要放宽的是把所有事件都当生成 delta，以及把 choice finish 当成拒绝后续元数据的唯一门槛。某种方言若明确允许语义终止后 EOF，可由该方言定义，但网络读取错误、缺少空行的残帧和无终止证据仍应失败；不得补成功尾标记掩盖截断。

流内错误也需独立解释。OpenRouter 会在普通 `data` 中提供顶层 error，而不仅是 `event: error`。Limen 已能识别这两类失败，但在 [SSE decoder](../modules/chat/src/sse.rs)第 73–86 行就丢弃错误载荷，业务适配器无法处理其结构。建议把上游错误作为有界响应数据交给明确的错误适配边界，保留失败终态；日志继续只记录清理后的分类，不能因可向客户端传递而记录正文。[OpenRouter streaming errors](https://openrouter.ai/docs/api/reference/streaming)

## 成熟网关提供的可借鉴边界

以下结论来自固定提交的源码核读，没有运行这三个项目的测试。

| 网关与版本                  | 已确认设计                                                                                         | 对 Limen 的价值与限制                                                                                                  |
| --------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| Envoy AI Gateway `ca3e5166` | 同协议请求在原始 JSON 上修改 model；响应观察模型和 usage，不要求重建整个响应                       | 原始载荷可作为传递依据，类型化视图用于观察和定点修改。该路径的 usage 解析较宽容，不能据此推断其严格验证全部 SSE 或截断 |
| Bifrost `767bd89a`          | passthrough 原样发送读到的字节，另用有界缓冲观察 usage；规范化链路另设 SSE reader 和 provider 解释 | 观察失败不必破坏原始转发。其 passthrough 普通 EOF 收尾，不等价于 Limen 所要求的生成成功判定                            |
| LiteLLM `2dc6aafa`          | 独立聚合器为文本、工具、推理、音频等设置合并逻辑，内部类型留有扩展出口                             | 可以借鉴字段语义和聚合分工；扩展出口不代表任意字段都可无损转换。wrapper 的部分收尾会补 `stop`，不宜照搬                |

源码：[Envoy 同协议 translator](https://github.com/envoyproxy/ai-gateway/blob/ca3e5166d9b6c491ea9acbcb9eaba2c6db1c4990/internal/translator/openai_openai.go)、[Bifrost passthrough](https://github.com/maximhq/bifrost/blob/767bd89a639efbe1461d7a653f451124ecd23931/core/providers/utils/passthrough_stream.go)、[Bifrost SSE reader](https://github.com/maximhq/bifrost/blob/767bd89a639efbe1461d7a653f451124ecd23931/core/providers/utils/sse.go)、[Bifrost 截断用例](https://github.com/maximhq/bifrost/blob/767bd89a639efbe1461d7a653f451124ecd23931/core/providers/openai/streamtruncation_test.go)、[LiteLLM 聚合器](https://github.com/BerriAI/litellm/blob/2dc6aafa6baf8dabdac58f5ba70c384b8dcb3e09/litellm/litellm_core_utils/streaming_chunk_builder_utils.py)、[LiteLLM streaming wrapper](https://github.com/BerriAI/litellm/blob/2dc6aafa6baf8dabdac58f5ba70c384b8dcb3e09/litellm/litellm_core_utils/streaming_handler.py#L1772)。

## 建议的架构边界

可以保留现有 transport、生命周期、VM 提交与资源管理主体，调整正文管线。没有证据要求重写整个网关。

```text
HTTP 原始状态、头、字节
  → 有界 JSON / SSE 分帧
  → 选定协议方言的解释与适配
  → 原始载荷 + 可理解字段视图 + 独立完成事实
  → provider / model 业务 hook
  → 同形转发，或有能力约束的聚合 / 合成流
  → 下游协议编码与错误终结
```

### 同形传递保留原始表示

原始 JSON 或 SSE frame 与解析视图并存。未改正文的同形路径保留载荷；模型别名或 hook 修改时，只重写相关 JSON，并保留外层 event/id/retry、其余扩展和顺序。别名改写意味着该路径不再保证字节完全相同，需明确区分载荷保留和字节透明。

不要要求所有可传递事件都先转为一个狭窄 ChatChunk。已理解字段仍可校验，但观测器不认识的字段，不应自动变成传递失败。关键错误和终止判断由选定方言承担；无法理解的语义应标为未知，不能用“透传成功”冒充“生成已完整完成”。

原始 frame 的保留仍须有大小上限，不能为了兼容引入完整流缓存。模型别名、统计和策略变换共用同一条执行骨架，避免另建一套与离线管线漂移的实现。原始字节快路径可后置，优先实现正确的事件级保留。

### 协议适配先于业务语义校验

当前 [receive_sse](../src/gateway/upstream.rs)第 224–258 行在 `onUpstreamEvent` 前已完成 decoder 和 tracker 校验；[event hooks](../src/gateway/events.rs)第 96–158、170–190 行也只接受 Chat chunk。因此仅扩大 JS hook 类型声明无效。

应明确区分协议适配和业务 hook：适配器接收有界原生 frame / JSON，把它解释为可保留原始表示的 Chat 方言事件；业务 hook 再处理已选管线的数据。可以区分生成 delta、用量快照、标注、附加元数据、错误和终止，避免让它们争用同一种 choice 状态转换。

请求端也需要相同边界：业务阶段保持客户端 Chat 请求契约，最后的协议编码阶段才构造上游请求。当前每个 `beforeSend` 后重新要求 model/messages 的 [校验](../src/gateway/pipeline.rs)第 145–183 行，不适合承担未来原生协议编码。此次只实现 Chat 方言；设计上留下该边界即可，不预先实现 Messages/Gemini，也不建立覆盖所有供应商的庞大通用中间模型。

客户端原始 stream 意图、上游原始状态和媒体类型仍须保留。解析方式由已选适配器和真实媒体事实决定，不能通过修改事实来切换解析器，也不依赖正文猜测。额外 JSON 媒体类型应按协议声明支持；出站 Accept 默认值宜结合最终上游 stream 计划生成，并保留配置覆盖能力。

### 聚合与合成流按字段语义定义

任意未知字段没有通用的拼接、深合并或最后值覆盖语义。扩大兼容不意味着取消转换检查。需要把当前封闭白名单改成有解释责任的转换规则：

| 字段类别                       | 合理处理方式                                                                              |
| ------------------------------ | ----------------------------------------------------------------------------------------- |
| 文本、refusal、字符串推理      | 按定义追加，同时保留缺失、null、空字符串的区别                                            |
| 工具与结构化 reasoning details | 按 choice / tool / detail 的标识组合；分别定义 ID、类型、名称、参数、签名与不透明块的规则 |
| logprobs                       | 按 token 序列累计，保留 bytes、候选和 refusal 信息                                        |
| 音频                           | 区分数据片段、转录、ID 和 expiry；不能把每个字符串都当普通文本追加                        |
| usage                          | 区分累计快照、增量和最终报告；累计值不能求和，也不能因正常增长判冲突；最终结果独立校验    |
| 标注与其他元数据               | 按其作用范围与协议定义保存；后置标注不重新打开生成状态                                    |
| obfuscation 等传输字段         | 明确标为仅属于流传输的字段；跨形可按规则剥离，不能靠通用未知字段丢弃来实现                |
| 无已知转换语义的扩展           | 同形保留；跨形要求适配器给映射，否则明确返回不支持，避免静默损失                          |

能力至少区分：可同形传递、可观察、可聚合为 JSON、可从 JSON 合成 SSE。能力可依协议、操作和模型确定，未知能力不能默认为拒绝同形传递。JSON → SSE 需要标明是完整响应的分段输出，不能承诺恢复 token 时间或原始事件次序。新字段出现时无需修改同形核心；需要跨形支持时才扩展相应语义规则。

### 完成、错误和资源保证继续独立维护

保留路由一次、provider → model 正序、客户端认证与上游认证独立、VM 线程所有权、取消后的提交失效、有界背压和统一终结预算。放宽语义形状无需放宽这些保证。

统计至少区分 observed usage、final usage、质量和上游完成事实。hook 可以改变对外输出，但不能改写原生完成事实；统计解析失败可标记观测质量，不应自动成为同形传递失败。明确网络错误和实际截断始终不能被 finish reason、usage 或 hook 补出来的成功尾块覆盖。

## 验证边界与实施评审依据

本轮运行的现有定向测试：`cargo test --locked --package limen-chat --lib`，43 项通过；`cargo test --locked --package limen --bin limen gateway::`，64 项通过。还执行了上述独立纯函数合成实验。现有测试通过与已复现缺口并不矛盾：部分测试正在固化原先较窄的契约。

未运行全 workspace 编译、Rusteward、脚本静态检查、CLI 离线用例、隔离 HTTP、真实供应商或 Docker 验收；本轮没有修改生产实现。外部网关仅完成静态源码核读。研究材料通过 Exa 多轮检索和页面读取取得，成功检索请求的 `numResults` 合计 59，含重复候选；结论采用一手资料，搜索摘要未单独作为最终证据。

评审重构时，先确定原始表示、方言适配和完成事实的契约，再迁移同形路径与 hook，最后扩展字段级转换器和统计语义。验收素材应按协议行为组织：网络分块不影响事件；注释和尾标注不丢失；音频 expiry 不重开生成；usage 增长不冲突；错误与残帧不成功；未知字段在同形保留、跨形有明确结果。常规合成离线管线与必要的隔离 HTTP 分层验证，沿用开发指引，不以源码布局断言代替行为测试。

这份报告不建立第二份进度清单。采纳后的持续契约应合入 `docs/design.md` 和响应协议专题，实际后续任务只记录在 `docs/roadmap.md`。
