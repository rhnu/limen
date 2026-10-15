# 响应与流协议

## 四种传输路径

下游形式取决于客户端最初的 `stream`，上游解析器取决于收到响应时的原始 `Content-Type`。`beforeSend` 改写上游 `stream` 不改变客户端意图；`afterReceive` 不能切换解析器。`Accept` 也不决定上游实际格式。

表中各 hook 阶段内部均按 provider → model 正序执行：

| 上游实际格式 | 客户端原始 stream | 正文处理顺序                                                       |
| ------------ | ----------------- | ------------------------------------------------------------------ |
| JSON         | `false`           | 完整 JSON → 别名 → `onResponse` → JSON                             |
| JSON         | `true`            | 完整 JSON → 别名 → `onResponse` → 转 chunk → `onStreamEvent` → SSE |
| SSE          | `false`           | 解码 chunk → `onUpstreamEvent` → 聚合 → 别名 → `onResponse` → JSON |
| SSE          | `true`            | 解码 chunk → `onUpstreamEvent` → 别名 → `onStreamEvent` → SSE      |

`onResponse` 不参与 SSE → SSE，`onUpstreamEvent` 不参与 JSON 上游，`onStreamEvent` 只在向客户端输出 SSE 时执行。别名只替换已有 `model` 字段；响应 hook 之后不再强制覆盖，因此脚本可明确设置对外模型名。

所有路径都先执行 `afterReceive`。接受的上游媒体类型为 `application/json` 与 `text/event-stream`，不区分大小写且允许参数；缺失、其他媒体类型、204/205 或无法解析的正文会失败，不根据内容猜格式。

## 同形传递与跨形转换

JSON → JSON 要求正文为合法 JSON 对象，保留扩展字段，不强制完整 Chat response schema。SSE → SSE 会解析、校验并重新编码 Chat chunk；不是原始字节透传，合法的同形扩展字段可保留。

JSON ↔ SSE 转换只接受当前实现能保真表达的结构：文本 `content`、`refusal`、function tool calls、多 choice、usage，以及支持的响应元数据。tool calls 按 choice 与工具 index 组合；ID、函数名和 arguments 的分片按顺序拼接，空 continuation 不抹除已有值。

转换拒绝未知结构，避免静默丢字段：例如多模态响应 content、未知扩展键、非 `null` 的 logprobs、非 function 工具或冲突元数据。相同格式能传递的扩展，不保证能跨形转换。需要适配时，在转换前的 `onUpstreamEvent` 或 `onResponse` 中明确整理结构。

JSON → SSE 按角色、内容、工具、完成原因和 usage 生成增量事件；超长文本与工具字符串仅在 UTF-8 字符边界切分。SSE → JSON 受累计输入事件数、累计字节和完整输出 JSON 限制。

可运行的四路径例子在 [`examples/testing/`](../../examples/testing/)，工具、扩展和截断回归素材在 [`fixtures/gateway/transport/`](../../fixtures/gateway/transport/)。

## SSE 完成与失败

数据事件必须是合法 `chat.completion.chunk`。choice index 在同一 chunk 中不能重复，tool index 在同一 choice 中不能重复，两者均为非负整数。`finish_reason` 必须为字符串或 `null`，每个已出现 choice 最终都要以非 `null` 字符串完成，完成后不能继续追加该 choice。所有 choice 完成后允许 `choices: []` 的 usage 尾事件。

上游必须提供由空行结束的 `data: [DONE]`；仅有 finish_reason 或直接 EOF 都算截断。网关在第一个有效 `[DONE]` 立即停止读取，忽略其后字节。注释、多行 data、UTF-8 BOM 和常见换行形式可以解析；非默认命名事件或上游 `event: error` 会失败。

下游成功时由 Rust 发送 `[DONE]`。失败发生在流启动前时，返回普通 JSON 错误；启动后 HTTP 状态已确定，尽可能发送 `event: error` 后结束，不发送成功 `[DONE]`。客户端断开时取消请求，错误帧不保证送达。

## 状态、响应头与错误

上游原始 2xx 走正文处理，`afterReceive.status` 只能映射为带正文的 2xx，不能映射为 204/205。上游原始 3xx/4xx/5xx 默认变为 502，可映射为 4xx/5xx；不能把上游失败改成成功。上游错误正文不透传，重定向不跟随。

响应媒体类型由 Rust 最终设置。JSON 使用 `application/json`，SSE 使用 `text/event-stream` 并设置 `Cache-Control: no-cache`；Limen 响应附带 `X-Limen-Request-Id` 便于关联日志。响应头 draft 的权限见[脚本文档](scripting.md#头部与客户端上下文)。

JSON 错误使用固定 envelope：

```json
{
  "error": {
    "code": "unknown_model",
    "message": "The requested model is not configured",
    "request_id": "limen-1"
  }
}
```

常见 code 与 HTTP 状态：

| 类别             | code                                                             | 默认状态                  |
| ---------------- | ---------------------------------------------------------------- | ------------------------- |
| 输入、路由、认证 | `invalid_request` / `unknown_model` / `unauthorized`             | 400 / 400 / 401           |
| 脚本策略拒绝     | `request_rejected`                                               | 400，可指定 403/422       |
| 容量与期限       | `payload_too_large` / `busy` / `deadline_exceeded` / `cancelled` | 413 / 429 / 504 / 499     |
| 脚本或内部故障   | `hook_failed` / `internal_error` / `unavailable`                 | 500 / 500 / 503           |
| 上游与协议       | `upstream_failed` / `upstream_protocol`                          | 502，前者可按上述规则映射 |

日志和观察 hook 额外提供 phase、step 与清理后的错误分类，不输出任意上游、脚本异常消息或正文。排错见[部署与运维](deployment.md#排错)。

协议权威实现：[`modules/chat/src/`](../../modules/chat/src/)、[`src/gateway/upstream.rs`](../../src/gateway/upstream.rs)、[`src/gateway/response.rs`](../../src/gateway/response.rs)。
