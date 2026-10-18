# JavaScript 脚本

## 模块与运行环境

`gateway.js` 必须是默认导出 `defineGateway()` 结果的 ESM。通过 `limen:gateway` 注册网关，通过 `limen:providers` 创建[内置 provider](configuration.md#内置-provider)，通过 `limen:policies`、`limen:content` 导入二进制内置工具，通过 `limen:runtime` 导入运行环境能力，不需要部署 SDK 运行时文件。也可用相对路径导入自己的本地 `.js`、`.mjs`，包括 `../`；本地模块按规范化文件路径加载并受总源码大小和模块数量限制。未知 `limen:` 模块、包名、`node:`、URL、带 query/fragment 的路径及 import attributes 均被拒绝。配置及其依赖在同一服务生命周期内保留，不随请求重新加载。

环境提供 ECMAScript 内建能力；读取环境变量使用 `import { env } from "limen:runtime"`，调用语义见[环境变量](configuration.md#环境变量)。不提供 Node.js、DOM、文件系统、网络或计时器 API。`async`/Promise 可用，顶层 await 也受启动预算约束；没有宿主 I/O 能唤醒的悬空 Promise 只能等待超时。需要 npm 库时先[打包为本地 ESM](#第三方库)。

## 显式组合与生命周期

`defineGateway(setup)` 描述一次启动注册。setup 的 `gateway` 用于创建 provider 和注册全局 hook；`provider.model()` 创建并返回所属模型的 scope。provider 与 model scope 都提供 `use(step)` 和 `step(name, setup)`，分别挂载工厂返回的 `PipelineStep` 和注册自定义阶段回调。用普通函数显式接收 scope，可将相关策略抽出复用：

```js
import { defineGateway } from "limen:gateway";
import { capCompletionTokens } from "limen:policies";

/** @param {import("limen:gateway").ProviderScope} scope */
function useProviderPolicy(scope) {
  scope.use(capCompletionTokens({ name: "cap-output", max: 4096 }));
  scope.step("remove-store", (step) => {
    step.beforeSend((ctx) => {
      delete ctx.body.store;
    });
  });
}

export default defineGateway((gateway) => {
  const primary = gateway.provider("primary", {
    endpoint: "https://example.invalid/v1/chat/completions",
  });
  useProviderPolicy(primary);
  primary.model("chat", { upstream: "demo-model" });
});
```

setup 与 step setup 都必须同步返回 `undefined`，不能是 async 函数或返回 Promise、scope、策略描述。`gateway.onRequest()`、`onErrorResponse()`、`onError()`、`onFinish()` 各最多注册一次；自定义 step 的同一阶段也只能注册一次。provider 标识唯一，模型别名在整个网关唯一，同一 provider 或 model 管线中的 step 名称唯一，`use()` 与 `step()` 共用该名称空间。不同 scope 可以使用相同 step 名称。

step scope 在自身 setup 返回后立即关闭；gateway、provider 和 model scope 在整个 setup 返回后关闭。保存 scope 到闭包后再在 Promise 或请求 hook 中注册会失败。setup 抛错、非法返回、重复注册、空 step 或无效配置都会使启动失败；不保留部分可运行的网关。默认导出普通对象不符合契约。

注册 scope 只描述启动配置，没有请求正文或请求状态。阶段回调的 `ctx` 则在每次请求调用时创建；业务输入、可写草稿和 `ctx.state` 不得存放在注册 scope 或模块级可变变量中。setup 完成后，宿主把注册结果验证并编译成请求执行计划，后续请求沿既定计划运行。

## Hook 顺序与可写内容

全局 `gateway.onRequest()` 在路由之前执行。路由后，每个阶段都遍历 provider 管线，再遍历 model 管线，各自按 `use()` 与 `step()` 的注册顺序正序执行；每个自定义 step 至少注册一个阶段。是否进入响应或事件阶段取决于[四种传输路径](transport.md#四种传输路径)。

| Hook              | 触发点                             | 可修改内容                                          |
| ----------------- | ---------------------------------- | --------------------------------------------------- |
| `onRequest`       | 认证、正文校验后，路由前           | `body`、`state`；可 `reject()`                      |
| `beforeSend`      | defaults 合并后，发送前            | `body`、`url`、`headers`、`state`；可 `reject()`    |
| `afterReceive`    | 收到上游头后，读正文前             | `status`、响应 `headers`、`state`                   |
| `onUpstreamEvent` | 每个真实上游 SSE JSON 数据事件     | `event`、事件替换、`state`                          |
| `onResponse`      | 完整 JSON，包括 SSE 聚合结果       | `body`、`state`                                     |
| `onContent`       | 正文中的语义事件，覆盖四种传输路径 | `event`、事件替换、`state`                          |
| `onStreamEvent`   | 每个即将发给客户端的 chunk         | `event`、事件替换、`state`                          |
| `onErrorResponse` | 失败已确定、错误响应尚未发出       | `status`、`body`、`contentType`、`headers`、`state` |
| `onError`         | 请求失败或取消后                   | 只读观察                                            |
| `onFinish`        | 每个终态，失败时位于 `onError` 后  | 只读观察                                            |

所有 hook 必须返回 `undefined` 或最终 resolve 为 `undefined` 的原生 Promise。通过修改 `ctx` 工作，不返回替代正文、事件、上下文或错误对象；`return ctx.body` 和 `return null` 都违反契约。

```js
provider.step("limit-output", (step) => {
  step.beforeSend((ctx) => {
    const requested = ctx.body.max_completion_tokens;
    ctx.body.max_completion_tokens = typeof requested === "number"
      ? Math.min(requested, 4096)
      : 4096;
    delete ctx.body.store;
  });
});
```

这里的 `provider` 是 setup 中创建的 provider scope；也可在 model scope 上注册同样的 step。

`body` 与 `event` 可直接修改或整体替换。`state` 引用固定，初始为 `{}`，其 JSON 字段可修改；不允许添加任意顶层上下文字段。调用成功且草稿通过校验后才有提交资格，throw、reject 的 Promise、非法返回值或非法草稿都不会提交当前调用的修改。`onContent` 还遵守下面的源正文批次提交规则。前序已提交的修改仍保留。

`beforeSend` 修改 `body.model` 或 `url` 只改变这次实际发送，不重新路由。不要用模块级可变变量保存请求数据：多个请求的 Promise 可交错完成，闭包副作用也不会因 hook 失败或取消回滚。

## 请求拒绝

在 `onRequest` 或 `beforeSend` 调用 `ctx.reject()`，默认返回 400；可指定 403 或 422。每次调用最多记录一次拒绝。它不会抛异常或提前结束函数，需要停止策略执行时自行 `return`：

```js
gateway.onRequest((ctx) => {
  if (ctx.client.headers.getAll("x-tenant").length !== 1) {
    ctx.state.reason = "tenant-required";
    ctx.reject(403);
    return;
  }
});
```

在网关 setup 中注册该回调，并配合下面的头部白名单。只有当前 hook 成功、草稿合法时拒绝才生效；此时有效 `state` 会提交，后续请求处理停止。throw、Promise 拒绝或破坏正文后再 `reject()` 会作为 hook 错误处理。默认返回固定 `request_rejected` 错误；自定义正文使用下面的错误响应适配。

## 错误响应适配

`onErrorResponse` 可通过 provider/model 的 `step.onErrorResponse()` 注册，也可通过 `gateway.onErrorResponse()` 注册全局回调。请求失败且响应尚未开始时，按 provider → model → 全局顺序执行；路由前失败只执行全局 hook。它与只读 `onError` 分开：适配完成后先发送错误，再由 `onError` 和 `onFinish` 观察最终失败。

```js
provider.step("normalize-errors", (step) => {
  step.onErrorResponse((ctx) => {
    if (ctx.meta.upstreamStatus === 429) {
      ctx.body = JSON.stringify({
        error: { code: "rate_limited", message: "Try again later" },
      });
      ctx.contentType = "application/json";
      ctx.headers.set("Retry-After", "30");
      ctx.state.rateLimited = true;
    }
  });
});
```

`ctx.status` 初始为实际待发状态；上游 HTTP 失败可映射为 3xx/4xx/5xx，网关错误仅可映射为 4xx/5xx。`ctx.body` 是原始正文的 UTF-8 字符串，JSON 错误可用 `JSON.parse`/`JSON.stringify` 适配。二进制、非 UTF-8 或带 `Content-Encoding` 的正文以 `null` 暴露；保持或设置为 `null` 表示保留当前原始字节。设置字符串表示显式替换正文，并移除原始 `Content-Encoding`。正文不要求是 JSON 对象，替换仍受 `max_output_bytes` 限制。

`ctx.contentType` 可设为合法头值或 `null`（移除媒体类型），`ctx.headers` 使用普通受控头 API。`ctx.error` 只读，包含清理后的原始失败 code、phase、step 和本次调用前的 HTTP status；原始网络、解析或 JS 异常不会暴露。`meta` 保留原始上游事实，`state` 字段可修改且引用固定。修改响应不改变请求失败分类，也不触发重新路由、重试或成功完成。

每次调用成功且整个草稿有效后，状态、头、正文、state 与观测一并提交。抛错、非法返回或非法草稿、超时会丢弃当前调用并停止后续适配，保留最后有效响应；不会递归触发错误 hook。没有有效适配时保留默认透传或网关 envelope。`onErrorResponse`、错误写出和两个观察 hook 共享 `cleanup_ms`，每个适配调用还受 `hook_ms` 限制。已超时的请求可在清理预算内适配错误，晚到结果无提交资格；VM 不可用时保留默认响应。已开始的 SSE 不执行此 hook，按[流错误规则](transport.md#sse-完成与失败)结束。

## 头部与客户端上下文

服务配置需显式允许脚本读取业务头：

```toml
[script_context]
client_headers = ["x-tenant", "x-example-policy"]
```

`ctx.client.headers.getAll(name)` 不区分头名大小写，返回新的字符串数组；缺失或未暴露时返回 `[]`。不支持写入，也不会自动转发。认证、Cookie 和传输相关头不能加入白名单；客户端 `Connection` 指名的头也会被剔除。要传递业务值，在 `beforeSend` 中明确设置：

```js
provider.step("forward-tenant", (step) => {
  step.beforeSend((ctx) => {
    const tenants = ctx.client.headers.getAll("x-tenant");
    if (tenants.length === 1) ctx.headers.set("X-Tenant", tenants[0]);
  });
});
```

在所需 provider 或 model scope 中注册该 step。`ctx.headers` 的 `getAll`、`append`、`set`、`delete` 操作也不区分大小写。`getAll` 返回副本，`append` 保留重复值，`set` 替换同名所有值。API 引用和方法固定，内容可以通过这些方法变更。

`Host`、`Content-Length`、`Content-Type`、`Content-Encoding` 及逐跳传输头由 Rust 管理，不能通过 headers draft 设置或删除；错误响应媒体类型用 `contentType` 修改。初始响应头保留上游 `Content-Language`、`X-Request-Id`、`Retry-After`、`WWW-Authenticate`、`Location`、`RateLimit`、`RateLimit-Policy` 以及 `RateLimit-*`、`X-RateLimit-*` 字段；`Connection` 指名字段剔除，不自动转发 Cookie 或认证凭据。响应 hook 可以再添加业务响应头。

## 元数据、状态与观察

`ctx.meta` 始终只读：`requestId` 和 `phase` 标识当前调用，`model` 是选中的别名，`provider` 是配置名，`configuredUpstreamModel` 是配置中的 upstream。路由前后三者由 `null` 变为已选值；`clientStream` 保留原始请求的流意图。收到上游头后，`upstreamStatus` 和 `upstreamContentType`（`json`/`sse`/`other`）固定为原始事实，不随 `afterReceive` 修改。

`onError` 与 `onFinish` 获得只读的 `state`、`meta`、`client`、`stats` 和终态 `outcome`。失败时附带经过清理的 `error`，含 code、HTTP status、request_id、phase 和 step。stats 提供字节数、上游响应头耗时、总耗时、实际发送模型与去除路径和 query 的目标 origin；没有正文。

观察 hook 与错误响应适配和写出共享有限清理预算。观察失败只产生诊断，不改变请求终态；前序工作耗尽预算或 VM 已退役时，后续观察可能不执行。不要依赖它们保证外部副作用完成。

### 原生用量与阶段耗时

`stats.usage` 在脚本处理前提取原始上游的 `usage`。JSON 在完整解析后采样；SSE 在 Chat 数据事件进入 `onUpstreamEvent` 前采样。脚本删除、替换或展开事件不会改变这些事实，网关也不自动设置 `stream_options.include_usage`。

| 字段                                           | 含义                                                                                                                          |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `source`                                       | `json`、`sse` 或尚未进入正文处理时的 `null`                                                                                   |
| `quality`                                      | `missing` 无有效计数；`partial` 核心三项不齐；`reported` 核心三项齐全；`invalid` 已知计数字段非法；`conflicting` 累计计数回退 |
| `upstreamCompleted`                            | 原始 JSON 完整解析，或原始 SSE 同时满足 `[DONE]` 和 choice 完成；不代表后续策略、转换或下游成功                               |
| `inputTokens` / `outputTokens` / `totalTokens` | 上游的 prompt、completion 和 total 计数                                                                                       |
| `cachedInputTokens` / `reasoningOutputTokens`  | 上游 details 中的缓存输入和推理输出子计数                                                                                     |

计数只接受非负安全整数，缺失为 `null`，零仍为零。缺失、空或 `null` 报告不会擦除已有值，累计 usage 按字段保留最新非递减快照，不按 chunk 相加。回退或非法报告会持续抑制全部有效计数。未知扩展不复制，缺失总数不估算，子计数不再次加入总数。统计异常本身不新增请求失败。`reported` 只说明计数齐全，需要结合 `upstreamCompleted` 判断协议是否完成；截断流也可能已报告 usage，缺失不能解释为未计费。

`stats.timings` 使用毫秒，保留亚毫秒小数；尚未进入的阶段为 `null`。新用量与耗时在请求终态确定时冻结，`onError` 和 `onFinish` 看到相同内容。既有 `elapsedMs` 仍在每次观察时采样。

| 字段                                           | 口径                                                                      |
| ---------------------------------------------- | ------------------------------------------------------------------------- |
| `requestMs`                                    | 网关开始处理请求至终态确定，不含之后的失败帧写入和终结观察                |
| `requestDecodeMs`                              | 请求正文读取、初始解析和校验                                              |
| `upstreamHeadersWaitMs`                        | 发起上游请求至响应头到达；旧 `upstreamHeadersMs` 仍从请求开始计时         |
| `upstreamFirstByteMs` / `upstreamFirstChunkMs` | 发起上游请求至首个非空正文片段、首个合法 Chat chunk；JSON 上游没有后者    |
| `downstreamFirstEventMs`                       | 请求开始至首个模型事件进入有界响应体，不是客户端实际收到的时间            |
| `upstreamReadWaitMs`                           | 等待上游读取的累计时间，不包含处理事件和等待下游的间隔                    |
| `downstreamQueueWaitMs` / `downstreamDrainMs`  | 等待输出通道和 SSE 响应体排空确认的累计时间；JSON 路径没有排空确认        |
| `hooks`                                        | 按业务 hook 阶段汇总调用、失败、中断、返回报告次数与 `wallMs`             |
| `native`                                       | 按原生策略阶段汇总 `calls`、`failedCalls`、`interruptedCalls` 和 `wallMs` |

每个 hook 汇总还有 `queue`、`initialCall`、`pending`、`inputBridge`、`outputBridge`，均为 `{ totalMs, samples }`。它们分别表示命令排队、初次函数调用、Promise 返回到终结观察、输入桥接、输出投影转换。`samples: 0` 表示没有采样，区别于实测零耗时。外层取消仍记录已经等待的 wall 时间和中断次数，无法取得的运行时细分不补零样本。原生策略不进入 VM，不产生这些 JS 细分样本。Promise 等待包含共享调度时间，不是该 hook 的 CPU 时间；里程碑与持续时间不能简单相加。终结观察自身的计时单独进入清理诊断，不放入其尚未结束时读取的请求快照。

## 受控观测

所有 hook 都有冻结的 `ctx.observe.record(name)`，只接受一个已注册事件名并返回 `undefined`。先在服务配置中显式批准这些非敏感元数据：

```toml
[observability]
events = ["output_tokens_capped", "request_finished"]

[limits]
max_observations_per_hook = 8
max_observations_per_request = 64
```

注册表默认空，最多 64 项；名称长 1–64 字节，匹配 `[a-z][a-z0-9_]*`，不得重复。未知名称、错参或修改冻结 API 会使本次 hook 失败，错误诊断不回显参数。`check` 验证注册表，但不执行 hook，因此不会提前发现动态调用中的未知名称。

```js
const observeLimit = {
  name: "observe-output-limit",
  beforeSend(ctx) {
    if (typeof ctx.body.max_completion_tokens === "number"
        && ctx.body.max_completion_tokens > 4096) {
      ctx.body.max_completion_tokens = 4096;
      ctx.observe.record("output_tokens_capped");
    }
  },
};
```

事件先进入当前调用的私有缓冲，只有 hook 成功、草稿满足网关阶段校验并提交时才生效。`onContent` 的观测与 state 一起等待整个源正文在当前 step 的语义批次通过校验。有效的 `reject()` 也会提交事件；throw、Promise 拒绝、非法草稿、取消和超时丢弃当前批次，包括其溢出计数。后续 hook 或请求失败不撤销前序已提交事件。`onError`、`onFinish` 可以记录事件，其余观察数据仍然只读；观察失败不覆盖主终态，也不保证后续观察能执行。停止等待 Promise 不会停止其 JS 工作，但晚到批次没有提交资格。

同一调用中已接受的事件名去重。`max_observations_per_hook` 默认 8，范围 1–64；`max_observations_per_request` 默认 64，范围 1–4096，覆盖主流程与终结观察。请求配额按每次 hook 提交的唯一事件次数计算，同组聚合仍消耗配额。超额项丢弃并累计 `dropped`，不改变业务结果；观测私有 envelope 同时受整个 hook 的 JSON 与 VM 预算约束。

终结后按事件名、phase、step 聚合次数，通过 `tracing` 输出有界的 `script observation` 元数据及必要的溢出计数，关联信息由 Rust 添加。接口不接受自由文本、任意标签、数值或 payload；注册名只应表达批准的非敏感事件。诊断进入滚动文件与 stderr，受日志过滤、队列容量和输出健康影响；usage 事实独立存入数据库。事件不是审计持久化或外部通知保证。离线测试可以[断言完整观测结果](testing.md#断言结果)。

## 统一正文变换

修改文本或工具参数等已知正文内容时，使用 pipeline step 的 `onContent`。同一套策略覆盖 JSON → JSON、JSON → SSE、SSE → JSON 和 SSE → SSE。完整 JSON 先执行 `onResponse`，再投影语义事件；原生 SSE 先执行 `onUpstreamEvent`，再投影，变换结果随后聚合或进入 `onStreamEvent`。SSE 聚合后的 `onResponse` 仍可改写完整原始对象，但不会再次执行 `onContent`。完整顺序见[四种传输路径](transport.md#四种传输路径)。

每个语义事件依次调用 `onContent(ctx)`。`ctx.event` 是下表中的普通 JSON 对象，可直接编辑或整体替换；`ctx.replaceEvents(array)` 每次回调最多调用一次，允许输出零到多个事件，默认输出编辑后的单个事件。替换结果按顺序进入下一个 step，不重新调用当前 step。`ctx.state` 可存有界的跨事件解析状态，引用固定。`ctx.sse` 是源 SSE 的只读 `event`、`id`、`retry` 信封；完整 JSON 和最终 `response_end` 的信封为 `null`。

| `type`         | 字段与含义                                                                                                                                                                                                 |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `part_start`   | `choice`、`part`、`kind`；kind 为 `text`、`reasoning`、`refusal` 或 `tool`。tool 时必须有 `tool: { index, type, id?, name? }`，type 为 `function`、`custom` 或 `legacy`                                    |
| `part_delta`   | `choice`、`part`、字符串 `text`；工具元数据可带 `tool: { id?, name?, type? }`。名称与 ID 可分段；尚未收到参数时允许明确为 function/custom。工具的 text 是原始参数或输入字符串片段，不承诺每片都是完整 JSON |
| `part_end`     | `choice`、`part`；有上游结束原因时带字符串 `finishReason`，用于在 choice 关闭前输出缓冲尾部                                                                                                                |
| `choice_end`   | `choice`、字符串 `finishReason`；之后不能继续向该 choice 生成内容                                                                                                                                          |
| `response_end` | 全部源正文成功完成；没有额外字段                                                                                                                                                                           |

`choice` 是非负整数，`part` 是当前 choice 内非空的稳定标识。不要解析或自行依赖生成的 part 名称格式。事件只允许对应的已知字段；delta 只能位于 start 与 end 之间，同一个 part 不能重复开始或结束。完整 JSON 在正文边界关闭 part；没有 finish reason 时不伪造 `choice_end`。需要保留片段尾部的策略应在 `part_end` 输出，不能等到 `choice_end` 或 `response_end` 再向已经结束的 choice 追加内容。

一次源正文是一个完整 JSON 对象或一个源 SSE 数据载荷。同一个 step 对它产生的多个语义回调顺序执行，后一次可读到前一次暂存的 state；全部事件合法、生命周期合法、渲染后正文及展开和编码预算均通过后，才一起提交 state 和受控观测。后面的回调或渲染失败会丢弃当前 step 整批修改，前序已提交的 step 和源帧保持有效。JS 闭包副作用仍无法回滚。

`response_end` 只在完整 JSON 合法完成，或 SSE 同时满足有效 `[DONE]` 与 choice 完成检查后发出。它允许更新 state 与记录观测，不能生成新的正文部件或代替 part 的收尾。EOF、读取错误、超时和取消不会产生成功结束通知，也不会帮策略刷新缓冲。`onFinish` 继续只读观察，不能用于补发内容。

每个 step 保留独立的输出生命周期，源投影与所有 step 的保留状态合计受 `max_json_nodes` 记录数和 `max_output_bytes` 保守字节预算限制；标识、工具类型、choice 及结束原因都会计入。完整源载荷的事件展开与输出另受原有累计预算约束，串联策略不会重置预算。建议只在 `ctx.state` 保存必要的解析状态；需要完整字符串时使用下述显式有界映射。

投影覆盖已知的文本、推理、refusal 和 function/custom/legacy 工具字符串叶子，不是覆盖所有供应商协议的通用模型。源布局与未知字段由 Rust 私有保留；同形变换只改写已理解的叶子，不经过 JSON → chunk → JSON 的转换白名单往返。未知内容块和附加字段可原样保留，不能通过语义事件任意构造它们；结构适配继续使用底层原始 hook。修改内容可能使标注、logprobs 或 offset 失去对应关系，因此同一响应中较早、当前或后续帧出现非空位置元数据都会与内容改写冲突并保守拒绝，不能把旧位置当成新正文的有效标注。

### 正文策略工厂

内置模块 `limen:content` 提供正文策略工厂。使用例子：

```js
import { replaceText } from "limen:content";

model.use(replaceText({
  name: "replace-brand",
  search: "old-name",
  replacement: "new-name",
  maxBufferedChars: 64,
}));
```

在 setup 中对所需 provider 或 model scope 调用 `use()`；内置模块名称不随配置所在目录变化。`replaceText` 与 `syntheticTextTool` 由 Rust 执行，请求状态由管线隔离，不进入 `ctx.state`。`mapPartText` 的用户回调由 JS 执行。完整选项与外部编辑类型见 [`sdk/modules.d.ts`](../../sdk/modules.d.ts)。

- `replaceText({ name, search, replacement, maxBufferedChars, maxParts? })` 只替换 text part，按字面量执行不重叠替换，支持匹配跨越多个 delta。`search` 非空；每个 part 最多保留 `search.length - 1` 个 UTF-16 单元，必填正整数 `maxBufferedChars` 必须覆盖该长度。尾部在 `part_end` 输出。
- `mapPartText({ name, kinds, maxBufferedChars, transform, maxParts?, stateKey? })` 缓冲所选 text/reasoning/refusal/tool part 的字符串，在 `part_end` 调用同步 `transform(text, info)`，返回替换字符串。`info` 只读，包含 choice、part、kind 和可选 finishReason。工具 ID/名称元数据仍及时传递，参数文字等待映射完成；全文正则或工具参数 JSON 编辑可以使用这一模式。每个 part 的保留文本受必填 UTF-16 长度上限约束，超限即失败；输出另受网关预算约束。该模式会延迟正文交付，流中断时不会调用映射函数或成功刷新缓冲。
- `syntheticTextTool({ name, toolName?, maxBufferedChars, maxPendingEvents?, maxParts? })` 追加文本输出 function 工具和一条传输指令消息，保留已有历史，将 `tool_choice` 设为 `required`、关闭并行工具调用。默认工具名为 `limen_emit_text`；该工具的参数必须是严格的 `{"text": string}` 对象，可有 JSON 空白，不能有额外或重复键。解析器增量解码合法 Unicode 文本，不缓存完整参数字符串；真实 function 工具调用继续传递，可与合成文本共存。移除合成工具后，保留的真实工具按 choice 重新连续编号，源 part 标识保持不变。只有出现合成文本且没有真实工具调用的 choice 才把 `tool_calls` 结束原因改为 `stop`。

工厂的 `maxParts` 默认 64。原生策略按管线位置自动隔离状态，不接受 `stateKey`；只有 `mapPartText` 保留该选项，用于隔离其 `ctx.state` 槽位，不要让不同脚本策略共享同一键。`syntheticTextTool` 的 `maxPendingEvents` 默认 64；`maxBufferedChars` 限制尚未判定工具身份时暂存事件的 JSON 字符串长度与工具元数据，必须至少覆盖工具名长度，不限制完整回答长度。所有输出仍受网关事件、JSON 和字节预算约束，原生缓冲也受请求总字节预算约束。

`syntheticTextTool` 在请求使用 legacy `functions`/`function_call`、`response_format`、非 1 的 `n`、`parallel_tool_calls: true`、非缺省/`auto` 的 `tool_choice`、非 function 工具或工具名冲突时拒绝 422。工具内容语法错误会失败；`length` 或 `content_filter` 允许在合法但未完成的 JSON 前缀处关闭，保留已经收到的文本和原结束原因，不补造后续字符。工厂不会在上游不遵循工具要求时重试或故障转移。

可运行配置与四路径用例见 [`examples/content/`](../../examples/content/)；执行入口见[离线测试](testing.md#运行示例)。

## 原始流事件替换

`onUpstreamEvent` 和 `onStreamEvent` 直接处理原始 Chat chunk，与完整对象的 `onResponse` 一起保留对未知字段和编码结构的适配能力。它们是独立能力。原始事件 hook 默认继续发送修改后的 `ctx.event`；调用 `ctx.replaceEvents(events)` 可丢弃、替换或按顺序拆分事件，每次调用最多一次：

```js
provider.step("drop-empty", (step) => {
  step.onUpstreamEvent((ctx) => {
    if (ctx.event.choices?.length === 0 && ctx.event.usage == null) {
      ctx.replaceEvents([]);
    }
  });
});
```

在所需 provider 或 model scope 中注册该 step。替换数组中的每个事件都会依次进入后续 step。`onUpstreamEvent` 的输出用于聚合或继续流处理；`onStreamEvent` 的输出只用于下游 SSE。不能操作 `[DONE]`，它由 Rust 在整体成功后发送。

`ctx.event` 是开放的 Chat JSON 对象，可能是内容增量、usage、标注或附加元数据。`choices`、`delta`、`finish_reason` 可能缺失，正文也可能是内容块数组；脚本读取时需要按形状判断。已完成 choice 可接收元数据尾事件，但不能继续生成正文、工具参数或推理内容。丢弃所有生成 choice 或必要的 finish 事件会失败。一个源事件经过所有事件 hook 的累计产出受 `max_output_events`、`max_output_bytes` 限制，不能逐步展开绕过限额。

事件 hook 还可读取 `ctx.sse`：原始 SSE 的 `event`、`id`、`retry` 只读字段；由完整 JSON 合成的事件为 `null`。修改 JSON 数据不会删除原始 SSE 外层字段。注释和无 data 帧直接作为存活信号传递，不进入业务事件 hook；错误和 `[DONE]` 由协议终结路径处理。

## JSON 边界

provider/model 选项、state、正文与事件只能包含普通 JSON 值。函数仅用于 setup、规定的阶段回调或工具工厂接受的回调。桥接不依赖用户自定义 `toJSON`，拒绝循环、Proxy、getter/setter、Symbol、BigInt、`undefined` 字段、稀疏数组、额外数组属性和自定义原型对象。数值必须有限，整数必须在 JS 安全整数范围内。可选字段应省略，不要用 `undefined` 或 `null` 代替缺失的配置项。

JSON 深度、节点和保守字节预算覆盖整个 hook 输出，不只覆盖正文。JS 中能构造的对象不一定能提交到 Rust，静态类型检查也不能替代这些校验。

## 第三方库

选择只依赖 ECMAScript 的库，先在 Node.js 开发环境将第三方依赖打包为本地 ESM，再交给 QuickJS。esbuild 使用 `external: ["limen:*"]` 保留内置模块导入，其余第三方依赖须打入产物。构建工具不会为运行时提供 `fetch`、`process`、`Buffer`、文件系统或 DOM；目标语法可编译不代表库的宿主能力可用。

随附的 [Valibot 示例](../../examples/libraries/src/gateway.js)通过谓词校验请求，在 `onRequest` 拒绝无效参数，在 `beforeSend` 实施 provider 限制；保留原正文可避免校验器丢失未知扩展字段。构建脚本见 [`build.mjs`](../../examples/libraries/build.mjs)，生成文件为同目录的 `gateway.js`，不要直接编辑它。重分发库代码时保留其许可证。

安装、构建、静态检查命令统一见[开发指引](../development.md#前端与-javascript)。打包后执行配置检查及[对应离线用例](testing.md#运行示例)，验证真实 QuickJS 行为。

## 可复用策略库

内置模块 `limen:policies` 提供请求策略工厂，返回原生 `PipelineStep` 描述，按原有 provider → model 顺序在 Rust 执行：

```js
import { capCompletionTokens, dropRequestFields, forwardClientHeader }
  from "limen:policies";

provider.use(capCompletionTokens({ name: "cap-output", max: 4096 }));
provider.use(dropRequestFields({ name: "remove-store", fields: ["store"] }));
provider.use(forwardClientHeader({ name: "forward-profile", from: "x-profile", to: "x-profile" }));
```

在 setup 中对所需 provider 或 model scope 调用 `use()`。工厂保存配置副本并冻结返回的描述，完整参数校验在配置启动检查中由 Rust 完成。一个 step 只能是原生策略或自定义 JS hook，不能向原生描述追加 hook；需要组合时依次调用 `use()` 与 `step()`。同一个工厂结果可供多个请求或管线位置复用。

| 工厂                                                    | 行为                                                                                                                                                                                                          |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `capCompletionTokens({ name, max, field?, onChange? })` | `max` 为正安全整数；默认操作 `max_completion_tokens`，可显式选择 `max_tokens`。选定字段缺失时设置 max，超过时裁剪；已有值必须是正安全整数，否则拒绝 422。另一种预算字段出现时也拒绝 422，不自动转换或选优先级 |
| `dropRequestFields({ name, fields, onChange? })`        | 删除指定自有字段，保留其他扩展。fields 必须为非空字符串数组，不得重复，禁止 model/messages/stream。配置数组之后被修改不会改变已创建策略                                                                       |
| `forwardClientHeader({ name, from, to })`               | 大小写无关地读取显式暴露的客户端业务头。缺失时保留原目标头，单值时 set，重复值拒绝 422。工厂拒绝非法、敏感及传输头名；读取仍受服务白名单和 Connection 指名剔除规则约束                                        |

name 遵循 pipeline step 名称规则，同一 pipeline 内不得重复。错误配置抛出固定 TypeError，不回显参数。前两个工厂的 `onChange` 可填写已注册观测名，只在实际设置、裁剪或删除时记录一次；它使用[受控观测](#受控观测)的提交与容量规则。静态类型能检查参数形状，但不能代替运行时数值、头名或注册表校验。

运行时工具随二进制提供，部署不需要 `sdk/`。外部类型声明仅供编辑器和类型检查使用。basic 示例直接导入内置模块；libraries 示例打包 Valibot 并保留 `limen:policies` 导入。回归用例见 [`fixtures/gateway/policies/`](../../fixtures/gateway/policies/)。

## 脚本静态检查

[`sdk/limen.d.ts`](../../sdk/limen.d.ts) 是 JSDoc/TypeScript 的可选编辑契约，会自动引用 [`sdk/modules.d.ts`](../../sdk/modules.d.ts) 中的 `limen:` 内置模块声明；它们都不是运行时模块。外部编辑环境保留这两份声明的相对位置，并将其纳入项目类型检查。`defineGateway()` 的回调可直接推导 scope 和请求 ctx；抽出的组合函数可显式声明 scope 类型：

```js
import { capCompletionTokens } from "limen:policies";

/** @param {import("limen:gateway").ProviderScope} scope */
function useOutputLimit(scope) {
  scope.use(capCompletionTokens({ name: "cap-output", max: 4096 }));
}
```

组合函数还可保留 provider 的 options 泛型和 hook 状态类型，供不同内置 provider 复用：

```js
/**
 * @template {import("../../sdk/limen").JsonObject} State
 * @template Options
 * @param {import("limen:gateway").ProviderScope<State, Options>} provider
 * @param {import("limen:gateway").ModelOptions<Options>} options
 */
function registerChat(provider, options) {
  return provider.model("chat", options);
}
```

工厂决定具体的 options 类型；例如 `createDeepSeek()` 返回的 scope 使 `registerChat()` 推导 `DeepSeekOptions`，不会将专有参数变成开放 JSON。选项覆盖和运行时验证见[内置 provider](configuration.md#内置-provider)。

完整用法见 [`examples/basic/gateway.js`](../../examples/basic/gateway.js)与[抽取策略的示例](../../examples/libraries/src/gateway.js)。声明使用 readonly 区分上下文权限，检查 setup、hook 返回值和已知字段；运行时仍验证注册生命周期、JSON 与业务契约。项目检查入口见[开发指引](../development.md#前端与-javascript)，hook 实现入口见 [`src/gateway/`](../../src/gateway/) 与 [`modules/script/src/context.js`](../../modules/script/src/context.js)。
