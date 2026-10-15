# 功能与使用指南

本文介绍 Limen 当前已实现的功能和配置方式。安装与快速开始见
[英文 README](../README.md) 或[中文 README](../README.zh-CN.md)，完整契约见[设计](design.md)，
开发检查命令见[开发指引](development.md)，尚未完成的验证只在[路线图](roadmap.md)中跟踪。

## 功能概览

Limen 将显式配置的模型别名映射到上游 Chat Completions 服务，并在请求和响应之间执行本地 JavaScript 变换。

| 功能           | 使用入口                           | 当前行为                                      |
| -------------- | ---------------------------------- | --------------------------------------------- |
| 模型列表       | `GET /v1/models`                   | 只列出 `models` 中的别名，不请求上游发现模型  |
| 对话生成       | `POST /v1/chat/completions`        | 接受模型别名与消息，支持 JSON 或 SSE 下游     |
| 客户端认证     | `server.yaml` 的 `client_auth`     | 两个接口共用 bearer token 或显式无认证配置    |
| 上游连接       | `config.js` 的 `providers`         | 完整 endpoint、独立认证、静态头与默认参数     |
| 模型映射       | `config.js` 的 `models`            | 选择 provider、上游模型名、模型默认参数和管线 |
| 请求与响应变换 | 全局 hook 和 `pipeline`            | 每个阶段按 provider → model 正序执行          |
| 流式变换       | `onUpstreamEvent`、`onStreamEvent` | 修改、丢弃或展开合法 Chat SSE JSON 事件       |
| 配置复用       | 本地 `.js` / `.mjs` ESM            | 相对导入、共享规则、同步或异步 hook           |
| 运行保护       | `limits`                           | 限制并发、队列、VM 执行、正文、流事件和聚合   |
| 部署与诊断     | CLI、日志、Docker Compose          | 配置检查、安全错误、非 root 容器和信号关闭    |

## 配置服务与模型

### 文件关系

一个配置入口由 YAML 与同目录的 `config.js` 组成：

```text
local/
├── basic/
│   ├── server.yaml
│   └── config.js
├── docker/
│   ├── server.yaml
│   └── config.js
└── shared/
    └── rules.mjs
```

`--config local/basic/server.yaml` 会定位 `local/basic/config.js`；文件名必须为 `config.js`。
模块中的相对导入以导入者为基准，允许 `../shared/rules.mjs`。
复制仓库示例时应复制整个 `examples/`，保留 `basic/`、`docker/` 与 `shared/` 的相对关系。

配置脚本作为可信本地代码执行。真实令牌可直接放在 `local/` 的配置文件或本地导入模块中，
也可通过 `limen.env(name)` 读取进程环境。限制秘密文件读取权限并排除版本控制。
无需 `package.json`，不支持 npm 包解析、远程 URL 导入、
Node.js、`fetch`、定时器、进程或任意文件读写 API。本地模块导入会读取对应文件。
模块图每次启动求值一次；修改配置后需要检查并重启服务。

### `server.yaml`

最小配置必须明确选择客户端认证方式。以下示例同时展示常用运行选项；省略的资源字段使用默认值：

```yaml
listen: 127.0.0.1:8080
client_auth:
  bearer: example-limen-client-token
limits:
  request_ms: 120000
  hook_ms: 5000
  concurrency: 32
  max_body_bytes: 4194304
log:
  level: info
  format: text
  output: stderr
```

`listen` 接受 IP 与非零端口组成的 socket 地址，默认 `127.0.0.1:8080`。
需要局域网访问时显式设置例如 `0.0.0.0:8080`，并配置客户端令牌。
无认证模式必须显式写为 `client_auth: {none: true}`，不能与 `bearer` 同时使用。
`bearer` 不是上游密钥；它用于客户端访问 Limen。

`log.level` 接受 `error`、`warn`、`info`、`debug`、`trace`，`format` 接受 `text` 或 `json`，
`output` 当前仅接受 `stderr`。基础配置拒绝未知字段、重复 YAML 键、anchor、alias、merge key 和多文档。

### `config.js`

以下完整示例包含一个 provider 和两个对外模型别名。地址和令牌均为占位值，
使用前请替换为实际服务的配置：

```javascript
export default {
  providers: {
    primary: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: "example-upstream-token" },
      headers: { "X-App-Name": "Limen" },
      defaults: { temperature: 0.7 },
      pipeline: [
        {
          name: "provider-compatibility",
          beforeSend({ body, headers }) {
            delete body.store;
            headers.set("X-App-Name", "Limen");
          },
        },
      ],
    },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "fictional-chat-model",
      defaults: { temperature: 0.4 },
      pipeline: [
        {
          name: "limit-output",
          async beforeSend({ body }) {
            const limit = body.max_completion_tokens;
            if (typeof limit === "number") {
              body.max_completion_tokens = Math.min(limit, 4096);
            }
          },
        },
      ],
    },
    fast: {
      provider: "primary",
      upstream: "fictional-fast-model",
    },
  },
};
```

Provider 字段为 `endpoint`、可选的 `auth`、`headers`、`defaults` 与 `pipeline`。
Model 字段为 `provider`、`upstream`、`defaults` 与 `pipeline`；模型的对象键就是对外别名。
`provider` 必须引用已定义项，`upstream` 为实际发送给上游的模型名。
`temperature` 和 `max_completion_tokens` 的可用性由具体上游决定，Limen 不为上游补充这些能力。

`endpoint` 必须包含完整 HTTP(S) 地址与实际路径，Limen 不追加 `/v1` 或 `/chat/completions`。
静态 query 可以保留，但 URL 不允许 userinfo、fragment、空白或反斜杠。
其他认证格式可以放入静态 `headers`；`auth.bearer` 与静态 `headers.Authorization` 同时出现会被拒绝。
配置记录拒绝未知字段，可选字段用省略表达缺失，不能用 `null` 替代配置对象或函数。

可配置 1–1024 个 provider、1–4096 个 model，每条 pipeline 最多 256 步。
Provider 名、模型别名与步骤名使用 ASCII 字母、数字或 `-._/`，长度为 1–128 字节。
同一条 pipeline 内步骤名必须唯一，每个步骤至少包含一个阶段函数。

### 从环境变量读取上游密钥

全局同步接口 `limen.env(name)` 可用于配置顶层、本地导入模块和 hook，返回 `string | undefined`。
例如将上游认证改为读取 `UPSTREAM_API_KEY`：

```javascript
const upstreamToken = limen.env("UPSTREAM_API_KEY");
if (!upstreamToken) throw new Error("UPSTREAM_API_KEY is required");

export default {
  providers: {
    primary: {
      endpoint: "https://fictional.invalid/v1/chat/completions",
      auth: { bearer: upstreamToken },
    },
  },
  models: { chat: { provider: "primary", upstream: "fictional-chat-model" } },
};
```

在启动 `check` 和 `serve` 前设置变量。POSIX shell：

```sh
export UPSTREAM_API_KEY='example-upstream-token'
./target/release/limen check --config local/basic/server.yaml
./target/release/limen serve --config local/basic/server.yaml
```

PowerShell：

```powershell
$env:UPSTREAM_API_KEY = 'example-upstream-token'
./target/release/limen.exe check --config local/basic/server.yaml
./target/release/limen.exe serve --config local/basic/server.yaml
```

以上令牌只是占位示例。仓库基础示例使用 `limen.env("UPSTREAM_API_KEY") ?? "example-upstream-token"`
保持未设置环境时仍可检查；真实接入应填写有效令牌，或按上例显式拒绝缺失。

未设置的变量返回 `undefined`，空字符串返回 `""`，不会自动 trim。
可选值可写成 `limen.env("UPSTREAM_MODEL") ?? "default-model"`；`??` 不替换空字符串。
变量名必须是非空字符串且不含 `=` 或 NUL，其他类型不隐式转成字符串，无效参数抛出 `TypeError`。
名称和值分别限制为 `limits.max_output_bytes` 个 UTF-8 字节，超过时抛出 `RangeError`；
非 Unicode 环境值抛出 `Error`。诊断不回显变量名、值或任意 JS 异常文本。

该接口只按名称读取，不提供枚举、写入或 `.env` 自动加载；`limen` 和 `limen.env` 不可替换或删除。
配置是可信本地脚本，可读取进程继承的任意环境变量。顶层读取的密钥随配置在启动时固定；
修改外部环境后需重启服务。`server.yaml` 不支持 `${...}` 插值，客户端 bearer 仍由 YAML 独立配置。

### 默认参数与单次路由

请求按以下顺序准备：

1. 校验客户端认证、JSON 基本形状，保存客户端原始 `stream`。
2. 执行全局 `onRequest`，用修改后的 `body.model` 选择模型和 provider。
3. 浅层合并 provider 默认值、model 默认值和当前请求体，越靠后优先级越高。
4. 将 `body.model` 初始化为所选模型的 `upstream`。
5. 执行 provider → model 的 `beforeSend`，校验并发送最终请求。

例如，上述配置中的 `chat` 在客户端省略 `temperature` 时使用 `0.4`；客户端提供 `0.2` 时使用 `0.2`。
嵌套对象与数组按顶层字段整体替换，不递归合并；显式 `null` 是已提供的值，不触发默认值。
`defaults` 禁止包含 `model`、`messages` 与 `stream`，这些字段需要用对应请求 hook 显式修改。

路由只发生一次。`onRequest` 可以更改用于路由的别名；`beforeSend` 可以修改最终上游
`body.model`、`body.stream` 与 `ctx.url`，这些修改不会重新选择 provider。
选中的 provider 认证先进入头部 draft，后续 hook 可以显式修改；发送前不会再静默注入一次。

## 调用 HTTP API

以下请求使用 POSIX shell 语法和占位令牌。令牌需与所选 YAML 中的 `client_auth.bearer` 一致。
PowerShell 使用 `curl.exe`，并调整换行和 JSON 引号；原样复制多行 POSIX 命令不能直接运行。

### 列出模型

```sh
curl http://127.0.0.1:8080/v1/models \
  -H 'Authorization: Bearer example-limen-client-token'
```

响应为 `{"object":"list","data":[...]}`，每项的 `id` 是配置中的别名，`object` 为 `model`，
`created` 为 `0`，`owned_by` 为 `limen`。未列出的模型不能直接透传到任意上游。

### 非流式生成

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}],"stream":false}'
```

请求必须是 JSON 对象，包含非空字符串 `model` 和非空对象数组 `messages`。
`stream` 可以省略，默认下游 JSON；提供时必须为布尔值。
消息内的多模态内容、工具定义和未知扩展字段默认保留，实际支持由上游决定。

### 流式生成

```sh
curl -N http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}],"stream":true}'
```

下游使用 `text/event-stream`。成功时最后一个控制帧为 `data: [DONE]`。
开始发送后失败时会收到 `event: error`，随后关闭连接；客户端应将其视为失败，并处理缺失 `[DONE]` 的情况。
请求 ID 通过 `x-limen-request-id` 返回，也包含在错误对象中，可用于关联日志。

配置兼容客户端时，Chat Completions base URL 使用 `http://127.0.0.1:8080/v1`，
API key 使用客户端 bearer token，model 使用对外别名。仅提供完整 URL 的客户端则填写
`http://127.0.0.1:8080/v1/chat/completions`。

## 编写 JavaScript hook

### 阶段与顺序

| 阶段              | 配置位置              | 可修改数据                           | 触发条件                    |
| ----------------- | --------------------- | ------------------------------------ | --------------------------- |
| `onRequest`       | 默认导出的顶层        | `ctx.body`                           | 认证后、路由前              |
| `beforeSend`      | provider/model 的步骤 | `ctx.body`、`ctx.url`、`ctx.headers` | 上游请求发送前              |
| `afterReceive`    | provider/model 的步骤 | `ctx.status`、`ctx.headers`          | 收到上游响应头后            |
| `onResponse`      | provider/model 的步骤 | `ctx.body`                           | 得到完整响应 JSON，最多一次 |
| `onUpstreamEvent` | provider/model 的步骤 | `ctx.event` 与替代事件集合           | 解析上游 SSE JSON 事件后    |
| `onStreamEvent`   | provider/model 的步骤 | `ctx.event` 与替代事件集合           | 输出下游 SSE JSON 事件前    |
| `onError`         | 默认导出的顶层        | 无，只读                             | 失败或取消的终态观察        |
| `onFinish`        | 默认导出的顶层        | 无，只读                             | 成功、失败或取消的终态观察  |

每个阶段先执行 provider pipeline，再执行 model pipeline，两者内部均按声明顺序。
响应阶段也保持正序。同一请求等待每个 hook 成功完成后再进入下一步；流事件也保持顺序。
不同请求可在异步等待时交错推进，共享 VM 的 JavaScript 字节码串行执行。

### 修改与返回值

普通 hook 修改 draft，完成值必须为 `undefined`，支持 `void | Promise<void>`。
不要返回修改后的对象、事件数组、`null` 或其他值；异步 Promise 也必须以 `undefined` 完成。
同步异常、Promise rejection、非 void 完成值、超时或非法 JSON 输出会使当前请求失败。
仅真正的 JavaScript Promise 用于异步完成，不支持自定义 thenable。

成功并通过校验后才提交 draft 与 `state`。删除 JSON 字段用 `delete`，不要将字段设置为 `undefined`。
输出只接受 JSON 数据，拒绝循环引用、函数、BigInt、NaN、Infinity、getter、Proxy 等结构。
HTTP JSON 的整数限制在 `±(2^53−1)` 安全范围内，深度与节点数也有限制。

`ctx.body` 和 `ctx.event` 可以原地修改或替换为合法对象。`meta` 与 `state` 的引用不能替换；
`state` 的内容可在普通 hook 中原地修改。URL、status 与 headers 只在对应阶段开放。

### 元数据与请求状态

`ctx.meta` 是只读数据，包含：

- `requestId` 与 `phase`：请求 ID 和当前阶段。
- `model`、`provider` 与 `configuredUpstreamModel`：所选别名、provider 和配置上游模型名。
- `clientStream`：客户端原始流式意图。
- `upstreamStatus` 与 `upstreamContentType`：原始上游状态和 `json` / `sse` / `other` 解析类别。

路由或响应尚未确定的值为 `null`。上游状态与媒体类型是事实，不会随响应 draft 修改而改变。
`ctx.state` 初始为空对象，在当前请求的阶段与事件之间保留，不与其他请求共享。
例如，在配置顶层加入以下 `onRequest`，即可将客户端模型名为 `default` 的请求显式路由到 `chat`：

```javascript
onRequest({ body, state }) {
  state.requestedModel = body.model;
  if (body.model === "default") {
    body.model = "chat";
  }
},
```

这是默认导出对象中的属性片段，`chat` 必须已在 `models` 中定义；它不会自动在模型列表中增加 `default`。
请求数据放在 `state` 中，不放入跨请求存活的全局变量或闭包。
所有异步工作都应纳入 hook 返回的 Promise，不启动脱离请求生命周期的后台任务。
超时或取消会停止等待并拒绝迟到结果，但不保证 Promise 本身或已发生的全局副作用已经停止。

### 请求头与响应头

`ctx.headers` 是大小写不敏感的受控 draft，不能替换引用，提供四种方法：

| 方法                  | 行为                 |
| --------------------- | -------------------- |
| `getAll(name)`        | 返回该头的值数组     |
| `append(name, value)` | 增加一个值           |
| `set(name, value)`    | 替换该头的全部现有值 |
| `delete(name)`        | 删除该头的全部值     |

在 provider 或 model 的 `pipeline` 中加入：

```javascript
{
  name: "request-metadata",
  beforeSend({ headers, meta }) {
    headers.set("X-Limen-Request", meta.requestId);
  },
  afterReceive({ headers }) {
    headers.set("X-Gateway", "Limen");
  },
},
```

上游头从 provider 静态头、provider 认证和默认 `Accept: application/json` 构建，
不复制客户端 Authorization、Cookie 或其他请求头。
`Host`、`Content-Length`、`Transfer-Encoding`、`Connection`、`Content-Encoding`、`Content-Type`
及其他逐跳传输头由 Rust 管理，配置或 hook 写入这些字段会失败。
可在 `beforeSend` 设置普通 `Accept`；最终 Content-Type 由实际传输形态决定。

下游初始只转发合适的 `content-language`、`x-request-id` 和 `retry-after`，
并排除上游 `Connection` 声明的逐跳头。`afterReceive` 可以修改普通下游头和允许范围内的状态。
原始成功状态只可映射到带正文的 2xx，禁止 204/205；原始 3xx–5xx 只可映射到 4xx/5xx。
状态修改不能让原始失败进入成功路径，也不能改变 parser 的选择。

### 修改完整响应

例如，在模型 pipeline 中加入以下步骤，为完整文本响应加前缀：

```javascript
{
  name: "prefix-response",
  onResponse({ body }) {
    for (const choice of body.choices ?? []) {
      if (typeof choice.message?.content === "string") {
        choice.message.content = "[Limen] " + choice.message.content;
      }
    }
  },
},
```

完整响应在进入 `onResponse` 前，存在的标准 `model` 字段默认改为对外别名。
Hook 可显式再次修改，发送时不强制恢复。JSON → SSE 转换保留此处已修改的模型名。
SSE → SSE 不因配置了 `onResponse` 而缓存整段流，该路径不会执行它。

### 修改、丢弃与展开流事件

流 hook 接收 `ctx.event`，默认输出修改后的当前事件一次。例如对文本 delta 做变换：

```javascript
{
  name: "uppercase-stream",
  onStreamEvent({ event }) {
    for (const choice of event.choices) {
      if (typeof choice.delta.content === "string") {
        choice.delta.content = choice.delta.content.toUpperCase();
      }
    }
  },
},
```

需要丢弃或展开时显式调用 `ctx.replaceEvents(events)`：

- `ctx.replaceEvents([])`：丢弃当前事件。
- `ctx.replaceEvents([eventA, eventB])`：按数组顺序输出两个替代事件。
- 未调用：输出当前 `ctx.event` 一次。

每次 hook 调用只能设置一次替代输出。替代事件继续进入本阶段剩余步骤，
不从第一步重新执行，仍须是合法 Chat chunk，包含协议所需字段。
不要误删 finish reason、tool call 分片或 usage 等必要信息；修改后仍会验证协议结束条件。
`[DONE]`、错误帧、heartbeat 和 SSE framing 由 Rust 管理，不作为 JSON 事件交给 hook 替换。
展开数量与字节预算累计整个源事件的成功 hook 输出，包括默认的一入一出，并非只计算最终数组。

### 终态观察

可在默认导出顶层配置 `onError(ctx)` 与 `onFinish(ctx)`，二者支持相同的 void/Promise 契约，
但整个 context 深度只读。提供 `meta`、最终 `state`、`outcome` 与 `stats`，失败时还有安全错误及阶段信息。
`stats` 包含输入、上游与输出字节数、耗时、最终上游模型及去除路径/query 的目标。

失败或取消时先尝试 `onError`，随后尝试 `onFinish`；观察失败只记录诊断，不恢复请求、不改响应、
不递归触发观察。它们共享有限的 `cleanup_ms` 预算；VM 已退役或预算耗尽时可能无法执行，日志会说明原因。
当前没有宿主日志或网络发送 API，观察 hook 不等于已提供外部遥测集成。

## JSON 与 SSE 转换

客户端原始 `stream` 选择下游格式。请求 hook 改写 `body.stream` 只影响上游请求；
Limen 根据上游真实 Content-Type 选择 parser，不根据请求猜测。

| 上游 | 下游 | 处理顺序                                                 |
| ---- | ---- | -------------------------------------------------------- |
| JSON | JSON | 完整 JSON → `onResponse` → 输出                          |
| JSON | SSE  | 完整 JSON → `onResponse` → 转换 → `onStreamEvent` → 输出 |
| SSE  | JSON | `onUpstreamEvent` → 有界聚合 → `onResponse` → 输出       |
| SSE  | SSE  | `onUpstreamEvent` → `onStreamEvent` → 输出               |

例如，客户端请求 `stream:true`，但 provider 的 `beforeSend` 设置 `body.stream = false`，
若上游返回 JSON，则走 JSON → SSE。客户端请求 JSON、hook 要求上游 SSE 时，则聚合后返回 JSON。
JSON → SSE 必须等待上游完整 JSON，不会缩短上游生成造成的首包等待。

跨形态转换支持文本、refusal、function tool calls（含 ID、函数名与 arguments 分片）、
按 index 区分的多 choice、finish reason 与 usage。
同形态保留合法扩展字段；跨形态遇到不支持的字段或结构明确失败，不能承诺任意多模态转换。

SSE 按事件解析，支持跨网络分块 UTF-8、多种换行与多行 `data:`。
`finish_reason` 不代表流已结束，其后仍可有 `choices: []` 的 usage chunk。
必须收到 `[DONE]` 才算成功；首个完整结束标记后停止读取，EOF 前缺少结束标记视为截断。
客户端断开会取消本地推进和上游读取，但不能保证上游生成或计费立即停止。

## 资源限制、错误与运行

### 调整资源限制

`limits` 中的值由 Rust 强制执行，hook 无法取消。常用默认值如下：

| 字段                   | 默认值   | 用途                                          |
| ---------------------- | -------- | --------------------------------------------- |
| `request_ms`           | 120000   | 整体请求预算，毫秒                            |
| `hook_ms`              | 5000     | 单 hook 的排队、等待与转换预算，毫秒          |
| `concurrency`          | 32       | 在途请求上限，观察清理期间仍占用许可          |
| `max_body_bytes`       | 4194304  | 客户端请求正文上限                            |
| `max_output_bytes`     | 4194304  | 合并请求、完整 JSON、桥接和事件展开的输出边界 |
| `max_event_bytes`      | 1048576  | 单 SSE 事件上限                               |
| `max_output_events`    | 64       | 单源事件的累计管线展开数量                    |
| `max_aggregate_bytes`  | 16777216 | SSE → JSON 聚合字节上限                       |
| `max_aggregate_events` | 100000   | SSE → JSON 聚合事件数上限                     |
| `cleanup_ms`           | 100      | 错误写出与终态观察的共享预算，毫秒            |
| `shutdown_ms`          | 5000     | 关闭等待预算，毫秒                            |

VM 内存、栈、执行与微任务工作量、命令队列、pending hook、模块源码图及 JSON 深度/节点
也有独立限制。完整字段与允许范围见[资源 schema](design.md#31-serveryaml进程与资源边界)。
这些默认值是资源边界，不是吞吐承诺；提高超时时仍需满足 schema 的范围。
请求许可不足返回 `busy`，不会自动重试或建立无限等待队列。

### 错误与排查

尚未开始发送响应时，错误为 JSON：

```json
{
  "error": {
    "code": "unknown_model",
    "message": "The requested model is not configured",
    "request_id": "1"
  }
}
```

`request_id` 是示意值。错误消息采用固定安全说明，不包含原始上游正文、脚本异常文本或令牌。

| code                                    | 默认 HTTP 状态 | 排查方向                                         |
| --------------------------------------- | -------------- | ------------------------------------------------ |
| `invalid_request` / `unknown_model`     | 400            | 请求 JSON、messages、stream 与模型别名           |
| `unauthorized`                          | 401            | 客户端 bearer token 与重复 Authorization         |
| `payload_too_large`                     | 413            | 客户端正文大小                                   |
| `busy`                                  | 429            | 请求并发或脚本队列容量                           |
| `hook_failed` / `internal_error`        | 500            | 阶段诊断、hook 完成值与 JSON 输出                |
| `upstream_failed` / `upstream_protocol` | 502            | 上游连接、状态、媒体类型、响应形状、预算或流截断 |
| `unavailable`                           | 503            | 服务关闭或 VM 状态                               |
| `deadline_exceeded`                     | 504            | 请求、hook 或等待预算                            |
| `cancelled`                             | 499            | 内部取消分类；客户端断开后不尝试回送             |

原始上游 3xx–5xx 默认映射为 502；`afterReceive` 可以在允许范围内显式改下游错误状态。
Limen 不跟随上游重定向、不自动重试、不使用环境代理，也不回显上游失败正文。
已经发送 SSE 后无法改 HTTP 状态，错误以 `event: error` 与相同 JSON 错误对象发送，然后关闭且不追加 `[DONE]`。

日志默认写 stderr，提供请求 ID、路由、阶段、状态、预算与结果元数据。
正文、令牌、Cookie、原始 query 和携密源码不进入日志；目标只记录 scheme、host 和 port。
诊断中的字段路径和位置也会清理，不通过开启 debug 来回显 JS 异常全文。

### 命令行与配置检查

```sh
limen --help
limen --version
limen check --config local/basic/server.yaml
limen serve --config local/basic/server.yaml
```

从源码运行可使用 `cargo run --locked -- check --config examples/basic/server.yaml`。
两个子命令的默认配置路径均为当前目录的 `server.yaml`，接受 `--config PATH` 与 `--config=PATH`。
`limen check --help` / `limen serve --help` 查看各自选项。

`check` 校验 YAML、本地模块、默认导出、provider/model 引用、静态认证冲突、管线与资源配置。
它会执行配置和导入模块的顶层 JavaScript，包括 top-level await；
不监听端口、不调用上游、不执行请求 hook，不能证明 hook 对所有输入都正确，也不能证明上游可用。

退出码为 `0`：检查成功、服务正常关闭、帮助或版本输出；`1`：配置、服务、运行时或关闭失败；
`2`：CLI 参数错误。修改配置后先检查再重启。
Ctrl-C 和 Unix SIGTERM 停止监听并取消在途请求；关闭超过预算会失败退出。
执行中断、失控微任务或宿主观测到的内存耗尽可能使共享 VM 退役，服务随后关闭并以失败退出，
不会重放在途生成请求。

## Docker 部署

仓库提供 Linux `amd64` 多阶段镜像构建与 Compose 配置。先复制整个 `examples/` 到 `local/`，
填写 `local/basic/config.js` 中的上游配置和 `local/docker/server.yaml` 中的客户端令牌：

```sh
docker compose build
docker compose run --rm limen check --config /config/docker/server.yaml
docker compose up -d
docker compose logs -f
docker compose down
```

`local/docker/config.js` 重导出 `../basic/config.js`，后者可以继续导入 `../shared/rules.mjs`。
Compose 将整个 `local/` 只读挂载到 `/config`，配置入口为 `/config/docker/server.yaml`。
容器内显式监听 `0.0.0.0:8080`，宿主默认仅发布 `127.0.0.1:8080`。
需要内网访问时同时考虑宿主端口绑定和客户端认证。

如果脚本使用 `limen.env("UPSTREAM_API_KEY")`，在 Compose 的 `services.limen` 下显式添加：

```yaml
environment:
  UPSTREAM_API_KEY: ${UPSTREAM_API_KEY:?set UPSTREAM_API_KEY before starting Compose}
```

设置宿主环境变量后再运行前述 `docker compose run` 和 `up`，两者会使用同一环境配置。
Compose 的变量替换发生在容器启动之前；宿主 `.env` 文件不会由 Limen 脚本自行加载。

运行用户为 UID/GID `10001`，挂载的 YAML、JS 和共享模块必须对其可读。
示例启用只读根文件系统、移除 capabilities、禁止提权、512 MiB 内存和 128 PID 上限，
终止宽限时间为 15 秒，`restart: unless-stopped`。自定义关闭预算时相应调整终止宽限时间。
真实配置不会进入允许清单控制的构建上下文，运行镜像保留 `/usr/share/doc/limen/LICENSE`。

CI 在 main push 检查通过后构建并推送 `ghcr.io/rhnu/limen:latest`；PR 与手动 CI 只构建。
包可见性与访问权限由 GHCR 设置决定，当前 Compose 默认从源码构建本地镜像。
镜像发布、缓存和历史版本清理细节见[开发指引](development.md#github-ci-与容器构建)。

## 当前支持边界

当前只提供 Chat Completions 与显式模型列表，不提供 UI、Responses、Embeddings、Images、音频 API、
自动重试、故障转移、热重载、上游模型发现、多租户或配额。
传递请求扩展字段不等于实现相应 API 或完整多模态跨形态转换。

Windows 和 Linux CI 已有纯逻辑测试、隔离 VM 与本地合成 HTTP 实验的运行证据，
Linux 镜像构建也已跑通。真实上游验收、Linux TLS/CA、目标环境慢客户端和流背压，
以及容器启动、挂载权限与关闭行为仍需单独验证；详见[路线图](roadmap.md)。
