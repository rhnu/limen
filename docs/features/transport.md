# 响应与流协议

## 四种传输路径

下游形式取决于客户端最初的 `stream`，上游解析器取决于收到响应时的原始 `Content-Type`。`beforeSend` 改写上游 `stream` 不改变客户端意图；`afterReceive` 不能切换解析器。`Accept` 也不决定上游实际格式。

表中各 hook 阶段内部均按 provider → model 正序执行：

| 上游实际格式 | 客户端原始 stream | 正文处理顺序                                                                     |
| ------------ | ----------------- | -------------------------------------------------------------------------------- |
| JSON         | `false`           | 完整 JSON → 别名 → `onResponse` → `onContent` → JSON                             |
| JSON         | `true`            | 完整 JSON → 别名 → `onResponse` → `onContent` → 转 chunk → `onStreamEvent` → SSE |
| SSE          | `false`           | 解码 chunk → `onUpstreamEvent` → `onContent` → 聚合 → 别名 → `onResponse` → JSON |
| SSE          | `true`            | 解码 chunk → `onUpstreamEvent` → `onContent` → 别名 → `onStreamEvent` → SSE      |

`onResponse` 不参与 SSE → SSE，`onUpstreamEvent` 不参与 JSON 上游，`onStreamEvent` 只在向客户端输出 SSE 时执行。别名只替换已有 `model` 字段；响应 hook 之后不再强制覆盖，因此脚本可明确设置对外模型名。

`onContent` 将源正文中的已知内容投影为语义事件，一轮管线覆盖四条路径。JSON 合成 chunk 和 SSE 聚合后的完整 JSON 不会再次触发它；纯用量、标注或未知元数据可能没有内容事件。逐事件结构、批次提交和有界策略见[统一正文变换](scripting.md#统一正文变换)。原始 `onResponse`、`onUpstreamEvent`、`onStreamEvent` 继续处理各自编码边界的开放对象。

所有路径都先执行 `afterReceive`。上游成功响应接受的媒体类型为 `application/json` 与 `text/event-stream`，不区分大小写且允许参数；缺失、其他媒体类型、204/205 或无法解析的正文会失败，不根据内容猜格式。上游 HTTP 失败走下面的错误响应路径。

## 同形传递与跨形转换

JSON → JSON 要求正文为合法 JSON 对象，保留扩展字段，不强制完整 Chat response schema。SSE 分帧与 Chat 解释独立；SSE → SSE 保留原始事件的命名、ID、retry、注释与未知 JSON 扩展。数据未变化时保留原始载荷，别名或 hook 改写时只重编码 data；改写后的字节不保证相同。

`onContent` 的同形渲染以私有源布局为模板，只修改已知内容叶子，保留其他字段，不借用跨形转换的字段白名单。语义投影并不扩大跨形转换能力；改写后的内容遇到依赖旧位置的非空标注、logprobs 或 offset 时保守拒绝，包括在后续帧才出现的位置元数据。

原生 SSE 的 `max_event_bytes` 按收到的完整帧计算；改写后的帧按实际编码字节检查，不用规范化 JSON 的大小替代原帧大小。JSON 解析在进入下一层前限制嵌套，支持配置允许的最大深度 128；节点数与安全数字检查仍作用于解析后的 JSON 树。

JSON ↔ SSE 转换按字段语义处理文本、内容块、`refusal`、字符串与结构化推理、function/custom 工具、legacy function_call、logprobs、音频、多 choice、累计 usage 和作用域明确的标注元数据。工具按 choice 与工具 index 组合；参数片段按顺序追加，空 continuation 不抹除已有值。累计 usage 保留最新快照而不求和，流专用 `obfuscation` 在跨形时移除。

内容块转换支持 text、thinking 和 refusal，保留块顺序及字符串与数组之间的转换。音频的独立 padded base64 片段先按字节合并再编码；未 padding 的连续片段按连续编码处理。

转换拒绝没有已知语义的扩展键或内容块，避免通用深合并或静默丢字段；固定元数据冲突仍然失败。多份不同的 filter/offset 快照无法保真压成完整响应中的单个字段，会明确拒绝跨形转换，同形仍保留全部事件。相同格式能传递的扩展，不保证能跨形转换。需要适配时，在转换前的 `onUpstreamEvent` 或 `onResponse` 中明确整理结构。

JSON → SSE 按字段生成增量事件，不恢复真实 token 时序。可拆分的文本在 UTF-8 边界切分，内容块数组按块拆分；不能安全拆分且超出事件上限的数据明确失败。SSE → JSON 受累计输入事件数、累计字节和完整输出 JSON 限制，不缓存完整事件历史。

可运行的四路径例子在 [`examples/testing/`](../../examples/testing/)，统一正文策略例子在 [`examples/content/`](../../examples/content/)，工具、扩展和截断回归素材在 [`fixtures/gateway/transport/`](../../fixtures/gateway/transport/)。

## SSE 完成与失败

数据事件是开放的 Chat JSON 对象，元数据和 `choices` 可省略；choice 的 `delta` 和 `finish_reason` 可省略，用于无生成内容的标注或元数据。choice index 在同一事件中不能重复，tool index 在同一 choice 中不能重复，两者均为非负整数。已观察的生成 choice 最终都要以非 `null` 字符串 `finish_reason` 完成。

完成后仍可接收标注、音频到期时间、usage 和相同 finish reason 的统计尾事件，无需附带特定供应商的 usage 形状。它们不重新打开生成状态。继续增加正文、refusal、工具、推理或音频数据，以及改变既定 finish reason，仍然失败。未知扩展可同形保留，但不能据此推断 choice 已完成。hook 改写后同样检查下游生成状态；原生上游完成事实独立保留。

上游必须提供由空行结束的 `data: [DONE]`；仅有 finish_reason 或直接 EOF 都算截断。网关在第一个有效 `[DONE]` 立即停止读取，忽略其后字节。注释、多行 data、UTF-8 BOM 和 CR/LF/CRLF 均受有界分帧处理。命名事件保留，数据仍按 Chat 解释；原生 Messages/Gemini 流不属于支持范围。

语义 `part_end` 在 `choice_end` 之前提供缓冲尾部的输出机会；choice 结束后不能重新生成。`response_end` 只有在有效 `[DONE]` 和 choice 完成校验后才产生，EOF、失败、超时和取消都不会触发成功刷新；只读 `onFinish` 不能补写正文。

注释和无 data 帧向 SSE 下游转发，JSON 下游聚合时忽略。首个存活帧会提交 HTTP 状态，后续错误只能流内表达；它们不触发业务事件 hook、不增加生成计数，也不延长总请求期限。

原始终止帧的注释、ID 等外层字段也在成功校验后保留。hook 展开时只保留一次流首 BOM；实际编码字节（包括外层字段）与 hook JSON 展开分别受 `max_output_bytes` 约束，无业务 hook 的存活帧与终止帧也检查输出上限。

上游 `event: error` 或普通 data 中非 `null` 的顶层 `error` 均终止为失败。合法有界的结构化原始错误保留为私有响应数据：流开始前可经 `onErrorResponse` 适配，开始后作为错误帧发出；非法错误载荷使用网关固定 envelope。错误载荷不进入日志或观察上下文，也不能被后续 `[DONE]` 覆盖。

上游 HTTP 2xx 的 JSON 对象包含非 `null` 的顶层 `error` 时同样按失败处理，在模型别名、正文 hook 和 JSON → SSE 转换之前保留原始错误正文，默认返回 HTTP 502，可经 `onErrorResponse` 适配。HTTP 200 本身不表示生成成功。

私有错误 JSON 保留原始数字和扩展字段，只检查字节、深度和节点上限，不套用 JavaScript 值桥接的安全整数限制。已开始的 SSE 错误直接对原始 JSON 数据分帧，避免数字或转义重新编码后的长度膨胀导致错误丢失。

下游成功时由 Rust 发送 `[DONE]`。失败发生在流启动前时，返回普通 JSON 错误；启动后 HTTP 状态已确定，尽可能发送 `event: error` 后结束，不发送成功 `[DONE]`。客户端断开时取消请求，错误帧不保证送达。

## 状态、响应头与错误

上游原始 2xx 走正文处理，`afterReceive.status` 只能映射为带正文的 2xx，不能映射为 204/205。上游原始 3xx/4xx/5xx 默认保留状态，可显式映射为其他 3xx/4xx/5xx，不能把上游失败改成成功。重定向不跟随。

上游 HTTP 失败的正文按字节透传，保留原始 `Content-Type` 与 `Content-Encoding`，不要求 JSON 或 Chat schema，不改模型别名；空正文、文本、HTML 和非 UTF-8 正文都可传递。即使客户端请求了流，也返回上游错误响应，不包装成成功 SSE。读取仍受请求期限与 `max_output_bytes` 限制；读取失败、超限或截断不能作为完整上游错误返回，使用相应网关错误。

成功响应媒体类型由 Rust 最终设置。JSON 使用 `application/json`，SSE 使用 `text/event-stream` 并设置 `Cache-Control: no-cache`；Limen 响应附带 `X-Limen-Request-Id` 便于关联日志。默认转发的元数据和响应头 draft 权限见[脚本文档](scripting.md#头部与客户端上下文)。

出站 `Accept` 在业务 hook 结束后按最终上游 `stream` 补默认值：流使用 `text/event-stream`，完整响应使用 `application/json`；配置或 hook 显式设置的值优先。该头不决定收到响应后的解析器。

网关自身错误默认使用固定 JSON envelope：

```json
{
  "error": {
    "code": "unknown_model",
    "message": "The requested model is not configured",
    "request_id": "limen-r-5d72071c-3ebc-44a2-b751-ec0d8ff8ee1a-1"
  }
}
```

常见 code 与 HTTP 状态：

| 类别             | code                                                             | 默认状态              |
| ---------------- | ---------------------------------------------------------------- | --------------------- |
| 输入、路由、认证 | `invalid_request` / `unknown_model` / `unauthorized`             | 400 / 400 / 401       |
| 脚本策略拒绝     | `request_rejected`                                               | 400，可指定 403/422   |
| 容量与期限       | `payload_too_large` / `busy` / `deadline_exceeded` / `cancelled` | 413 / 503 / 504 / 499 |
| 脚本或内部故障   | `hook_failed` / `internal_error` / `unavailable`                 | 500 / 500 / 503       |
| 持久化不可用     | `storage_unavailable`                                            | 503                   |
| 上游 HTTP 失败   | 内部归类为 `upstream_failed`，正文保留上游格式                   | 上游原状态            |
| 网络与协议       | `upstream_failed` / `upstream_protocol`                          | 502，超时为 504       |

请求总超时与单次 hook 超时都使用 `deadline_exceeded` 和默认 HTTP 504，但默认文案区分来源：

| 超时来源                            | `error.message`                     |
| ----------------------------------- | ----------------------------------- |
| 请求总期限                          | `The request deadline was exceeded` |
| 单次 JS hook 等待或原生策略执行预算 | `The hook deadline was exceeded`    |

请求总期限先到时，即使正在等待 hook，也使用请求超时文案。默认期限与配置范围见[配置与接入](configuration.md#配置文件)。SSE 已开始时沿用相同错误文案发送流内错误，不能改写已提交的 HTTP 状态；未开始的错误响应仍可由 `onErrorResponse` 显式适配。

需要统一供应商错误格式或自定义网关错误时，配置 [`onErrorResponse`](scripting.md#错误响应适配)。它可修改尚未发出的错误响应，保留失败分类；已经开始的 SSE 仍按流内错误规则结束。

失败日志通过 `requestId` 关联请求，`errorDiagnostic` 保留类型化安全原因链，例如缺少 `[DONE]`、choice 未结束、SSE UTF-8 错误、事件超限，以及 JSON 错误类别和行列位置。上游结构化错误的 `error.code` 为 400–599 整数时另记 `upstreamErrorStatus`，它与实际 HTTP `upstreamStatus` 独立。任意上游 message、字符串 code、正文、认证信息及原始网络或脚本异常文本不进入日志；完整的合法有界上游错误仍沿客户端响应路径保留，不因日志脱敏而替换。观察 hook 继续只接收受控错误分类。排错见[部署与运维](deployment.md#排错)。

协议权威实现：[`modules/chat/src/`](../../modules/chat/src/)、[`src/gateway/upstream.rs`](../../src/gateway/upstream.rs)、[`src/gateway/response.rs`](../../src/gateway/response.rs)。
