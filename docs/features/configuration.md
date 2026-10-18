# 配置与接入

## 配置文件

Limen 从服务配置所在目录加载 `gateway.js`。服务配置负责监听、客户端认证、日志、观测事件注册和资源限制；JavaScript 默认导出 `defineGateway()` 创建的网关工厂，在启动时显式注册 provider、模型别名与 hook。

```text
local/
  config/
    server.toml     # YAML/YML 需要显式 --config
    gateway.js
  data/
  logs/
```

从仓库运行时，也可将 `examples/basic/server.toml` 与 `examples/basic/gateway.js` 复制到 `local/config/`，修改客户端 token、上游 URL、独立上游凭据和模型名，并让 `--config` 指向 `local/config/server.toml`。basic 示例通过 `limen:policies` 导入二进制内置策略，不需要复制 SDK 运行时文件；外部类型声明只用于编辑与静态检查。

未指定 `--config` 时，只读 `./config/server.toml`，不存在直接报错。YAML/YML 必须通过 `--config` 显式选择，不探测旧目录或其他扩展名。配置按扩展名解析，必须是 UTF-8。

最小 `server.toml`：

```toml
listen = "127.0.0.1:8080"

[auth]
api_key = "replace-with-a-random-api-key-at-least-32-bytes"
# manager_key = "replace-with-a-different-random-manager-key"
```

`auth.api_key` 必填，`auth.manager_key` 缺省表示管理 API 关闭。每个 key 必须为 32–256 字节、无空白的可打印 ASCII，且二者不同；空字符串无效。旧 `client_auth`、匿名模式与 `log.output` 已移除，不提供兼容字段。TOML 与 YAML 使用同一字段结构，见 [TOML 示例](../../examples/basic/server.toml)和[YAML 示例](../../examples/basic/server.yaml)。未知字段、重复键、无效类型直接拒绝；YAML 不接受多文档、锚点、别名、合并键或自定义标签。

运行参数的默认值：

```toml
[limits]
request_ms = 600000 # 请求总期限：10 分钟
hook_ms = 5000 # 单次 hook 等待：5 秒

[storage]
dir = "../data"
retention_days = 90
max_records = 100000
max_db_bytes = 1073741824
operation_ms = 1000

[log]
level = "info"
format = "text" # stderr；文件始终为 JSONL
dir = "../logs"
max_file_bytes = 16777216
max_files = 8 # 备份数量，另有活动文件

[management]
concurrency = 4
timeout_ms = 2000
```

请求总期限从生成任务开始计时，覆盖正文读取、hook、上游调用及响应处理，SSE 的下游发送也计入期限；收到 token 或心跳不会续期。单次 hook 等待包含排队与 Promise 等待，实际截止时刻取 `hook_ms` 与请求剩余期限的较早者；请求总期限先到时按请求超时报告。两类超时的默认错误文案见[状态与错误](transport.md#状态响应头与错误)。`request_ms` 可设为 1–3600000 毫秒，`hook_ms` 可设为 1–300000 毫秒。

相对目录以服务文件父目录为基准。配置、数据、日志目录不能相同或互相包含；已有路径的符号链接会解析后检查。`serve` 创建缺失的 data/logs，但不创建配置或修改权限。`check` 不打开数据库、不创建日志或运行目录，通过也不代表目录可写。`storage.operation_ms + limits.cleanup_ms` 必须小于 `limits.shutdown_ms`，并应为排空留余量；记录上限必须大于生成并发。完整范围见[设计契约](../design.md#服务配置与命令)。

`gateway.js` 示例：

```js
import { requiredEnv } from "limen:runtime";
import { defineGateway } from "limen:gateway";

export default defineGateway((gateway) => {
  const primary = gateway.provider("primary", {
    endpoint: "https://api.example.com/v1/chat/completions",
    auth: { bearer: requiredEnv("UPSTREAM_API_KEY") },
    defaults: { temperature: 0.7 },
  });
  primary.model("chat", {
    upstream: "your-upstream-model",
    defaults: { temperature: 0.4 },
  });
});
```

将 endpoint 和 upstream 改为实际服务值。endpoint 是完整 Chat Completions URL，Limen 不追加路径；只接受绝对 HTTP(S) URL，不接受 userinfo 或 fragment。provider 与 model 都需至少一项；通过 provider scope 注册 model，归属由 scope 确定。provider 标识与全局模型别名分别唯一。

启动前先验证，再运行服务：

```sh
cargo run --locked -- check --config local/config/server.toml
cargo run --locked -- serve --config local/config/server.toml
```

`check` 会执行顶层 JS、导入模块、调用同步 setup 并验证注册结果，不调用请求 hook、不监听端口、不请求上游。服务只在启动时加载配置；修改后需重启。setup 与 `step` 注册回调必须同步返回 `undefined`；默认导出普通对象、异步 setup、重复注册或 setup 后追加策略都会失败，详见[显式组合与生命周期](scripting.md#显式组合与生命周期)。

## 内置 provider

`limen:providers` 提供 `createOpenAI`、`createDeepSeek` 和 `createOpenRouter`。工厂显式接收 gateway、provider 标识和连接选项，返回支持 `model()`、`use()`、`step()` 的 provider scope。普通兼容端点仍可通过 `gateway.provider()` 注册。

```js
import { defineGateway } from "limen:gateway";
import { createDeepSeek } from "limen:providers";
import { requiredEnv } from "limen:runtime";

export default defineGateway((gateway) => {
  const deepseek = createDeepSeek(gateway, "primary", {
    apiKey: requiredEnv("DEEPSEEK_API_KEY"),
    options: { thinking: "enabled" },
  });
  deepseek.model("reasoner", {
    upstream: "deepseek-v4-pro",
    defaults: { max_tokens: 8192 },
    options: { reasoningEffort: "high" },
  });
});
```

三家工厂都要求显式提供非空 `apiKey`，不会读取默认环境变量或客户端凭据。上例通过 `requiredEnv` 在环境变量缺失或为空时使启动失败。可选的 `endpoint` 替换完整 URL，不追加路径；`headers` 添加静态上游头；`defaults` 保留开放的上游 JSON 字段；provider 级和 model 级 `options` 分别提供有类型的专有默认参数。

| 工厂               | 缺省 endpoint                                   | 额外连接选项                                                         |
| ------------------ | ----------------------------------------------- | -------------------------------------------------------------------- |
| `createOpenAI`     | `https://api.openai.com/v1/chat/completions`    | `organization` → `OpenAI-Organization`；`project` → `OpenAI-Project` |
| `createDeepSeek`   | `https://api.deepseek.com/chat/completions`     | 无                                                                   |
| `createOpenRouter` | `https://openrouter.ai/api/v1/chat/completions` | `appUrl` → `HTTP-Referer`；`appTitle` → `X-OpenRouter-Title`         |

专有 options 在启动时展开为原生请求字段，不在客户端正文增加包装层，也不安装隐藏的请求 hook。未提供的 option 不会补供应商默认值；模型仍需显式注册，上游对具体模型支持哪些参数负责。

| options 类型        | 首版字段与展开                                                                                                                            |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `OpenAIOptions`     | `reasoningEffort` → `reasoning_effort`；`verbosity`；`serviceTier` → `service_tier`；`store`；`parallelToolCalls` → `parallel_tool_calls` |
| `DeepSeekOptions`   | `thinking: "enabled" \| "disabled"` → `thinking: {type: ...}`；`reasoningEffort` → `reasoning_effort`                                     |
| `OpenRouterOptions` | `provider` 路由偏好与 `reasoning` 推理控制对象，见下文                                                                                    |

OpenAI 与 DeepSeek 的 `reasoningEffort` 接受 `none / minimal / low / medium / high / xhigh / max`；具体模型不一定支持全部值。OpenAI 的 `verbosity` 接受 `low / medium / high`，`serviceTier` 接受 `auto / default / flex / scale / priority / fast`，`store` 与 `parallelToolCalls` 为布尔值。字段依据见 [OpenAI Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create) 与 [DeepSeek Chat Completions](https://api-docs.deepseek.com/api/create-chat-completion/)。

OpenRouter 的 `provider` 接受字符串数组 `order / only / ignore`，布尔值 `allowFallbacks / requireParameters / zdr`，`dataCollection: "allow" | "deny"`，以及 `sort: "price" | "throughput" | "latency"`；展开时将 camelCase 字段改为上游的 snake_case。`reasoning` 接受 `effort`（与上述推理强度取值相同）或正安全整数 `maxTokens`（展开为 `max_tokens`），二者互斥；还可提供布尔值 `enabled / exclude`。这些参数只转交 OpenRouter，不在 Limen 中执行路由或故障转移，Limen 仍只发送一次上游请求。依据见 [OpenRouter 路由偏好](https://openrouter.ai/docs/guides/routing/provider-selection)、[推理控制](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)与[应用标识](https://openrouter.ai/docs/app-attribution)。

工厂配置与已知 options（包括嵌套对象）拒绝未知字段、非法类型、非法枚举及 accessor。`headers` 与 `apiKey` 或额外连接选项生成的同名头按大小写无关规则判冲突；URL、头部限制、开放 defaults 的 JSON 与保留字段仍由宿主检查。同一层的 options 与 defaults 写入同一上游顶层字段也会使启动失败，即使值相同；例如 `options.reasoningEffort` 不能与该层 `defaults.reasoning_effort` 并存。

展开后沿用[模型别名与默认值](#模型别名与默认值)的浅合并与优先级：客户端正文 > model > provider。`options` 表达可覆盖的默认值，需要强制限制时显式挂载策略。OpenRouter 的 model 或客户端只要提供自己的 `provider` 对象，就会整体替换低优先级对象，不会保留其中的隐私或路由设置。

`ModelOptions<Options>` 与 `ProviderScope<State, Options>` 支持泛型；三家工厂分别返回绑定上述 options 类型的 scope，同时保留 hook 的 `State` 推导。普通 `gateway.provider()` 的 options 类型为 `never`，不接受未经展开的 `options`。泛型用于复用类型声明和组合函数，未封装的上游扩展字段通过 `defaults` 传递。

完整类型见 [`sdk/modules.d.ts`](../../sdk/modules.d.ts)。[`examples/providers/`](../../examples/providers/)同时演示三家工厂、完整 endpoint 覆盖、模型默认值、专有参数和显式输出限制；其中凭据、上游模型和域名都是合成值，可直接运行[离线用例](testing.md#运行示例)。

## 环境变量

从 `limen:runtime` 导入 `env`，通过 `env(name)` 读取进程环境变量。变量不存在时返回 `undefined`，存在但为空时返回 `""`；`??` 只处理前者。名称必须非空且不含 `=` 或 NUL，名称与值受字节限制，非 Unicode 值会抛错。

必需的凭据使用同一模块导出的 `requiredEnv(name)`：返回类型为 `string`，变量缺失或为空字符串时抛出 `Error`。它与 `env` 共用名称校验、字节限制和非 Unicode 错误处理；非法名称抛出 `TypeError`，超过字节限制抛出 `RangeError`。两者均不裁剪值，纯空白仍是非空字符串，凭据的其他约束由使用方校验。错误消息不包含变量名或值。顶层或同步 setup 中未捕获的异常会使启动失败；hook 中的异常沿现有阶段失败与事务提交规则处理。

变量来自启动 Limen 的进程环境，服务 TOML/YAML 不做环境变量插值，也不自动读取 `.env`。可在顶层读取一次保存到闭包，或在 hook 中按需读取。应由 shell、服务管理器或[容器配置](deployment.md#docker)提供变量，避免写入受版本管理的文件。

## 模型别名与默认值

客户端提交 `model: "chat"`。`onRequest` 执行后，Limen 按别名查找一次 model，再选定其 provider；未知别名返回 `unknown_model`。

默认值只补充缺失的顶层字段，优先级为：

```text
客户端正文（含 onRequest 修改） > model.defaults > provider.defaults
```

合并是浅合并；已有对象不会递归补全，显式 `null`、`false` 和 `0` 都是已提供的值。defaults 禁止包含 `model`、`messages`、`stream`。随后正文中的 `model` 被设置为 `model.upstream`，再进入 `beforeSend`；需要强制覆盖参数时使用 hook。

响应原有的 `model` 字段会在[响应阶段](transport.md#四种传输路径)替换为选中的客户端别名，后续响应 hook 可以明确改写它。

## 认证与请求头

客户端使用服务配置中的 token：

```http
Authorization: Bearer replace-with-a-random-api-key-at-least-32-bytes
```

apiKey、managerKey 与 `provider.auth.bearer` 分别管理。客户端认证头不会自动转发，脚本也不能通过客户端白名单读取 `Authorization`、Cookie 等敏感头。

上游请求头由 `provider.headers`、`provider.auth` 和 `beforeSend` 构造。`auth` 与静态 `Authorization` 不能同时配置，静态头名不区分大小写且不得重复。未提供 `Accept` 时默认 `application/json`；出站正文始终为 JSON，`Content-Type` 由 Rust 设置。传输头由 Rust 管理，脚本不可改写，详见[头部与上下文](scripting.md#头部与客户端上下文)。

脚本要输出批准的观测事件时，先在服务配置的 `[observability].events` 注册名称，再调用请求级 API；默认注册表为空，详见[受控观测](scripting.md#受控观测)。

## API

`/v1/*` 使用 apiKey，`/admin/v1/*` 使用 managerKey，二者不可互换；管理请求不经过 JS。已知命名空间先认证再分派方法和路径，未知顶层路径直接 404，管理未启用时管理路径为 404。只接受单个 Bearer Authorization，不接受 query 或 Cookie 凭据。

| 方法与路径                  | 用途                               |
| --------------------------- | ---------------------------------- |
| `GET /v1/models`            | 返回显式配置的模型别名，不查询上游 |
| `POST /v1/chat/completions` | 运行 Chat Completions 管线         |

Chat 请求需使用 `Content-Type: application/json`，可附带媒体类型参数。正文必须是对象，`model` 是非空字符串，`messages` 是非空对象数组，`stream` 若存在必须是布尔值，缺省为 `false`。网关不为每个 provider 验证完整消息 schema；多模态内容、工具和扩展字段仍作为普通 JSON 传递，可由脚本施加更严格策略。

在支持 Chat Completions 的客户端中，将 base URL 设为 `http://127.0.0.1:8080/v1`，API key 填服务配置的客户端 token，model 填配置别名（如 `chat`）。

向已启动的服务发送最小请求：

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer replace-with-a-random-api-key-at-least-32-bytes' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}]}'
```

流式请求添加 `"stream":true` 并使用 `curl -N`；PowerShell 使用 `curl.exe`。响应和错误边界见[响应与流协议](transport.md)。

权威字段与校验入口：[`sdk/limen.d.ts`](../../sdk/limen.d.ts)、[`modules/config/src/server.rs`](../../modules/config/src/server.rs)、[`modules/config/src/business.rs`](../../modules/config/src/business.rs)。
