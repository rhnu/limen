# 配置与接入

## 配置文件

Limen 从服务配置所在目录加载 `config.js`。服务配置负责监听、客户端认证、日志、观测事件注册和资源限制；JavaScript 默认导出的对象负责 provider、模型别名与 hook。

```text
local/
  config/
    server.toml     # YAML/YML 需要显式 --config
    config.js
  data/
  logs/
```

也可将整个 `examples/` 与 `policies/` 一起复制到 `local/` 下，从 `local/examples/basic/` 开始修改客户端 token、上游 URL、独立上游凭据和模型名，并让 `--config` 指向 `local/examples/basic/server.toml`。basic 示例导入 `../../policies/index.js`，需保留这层相对关系；不能只复制 `config.js`。

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

相对目录以服务文件父目录为基准。配置、数据、日志目录不能相同或互相包含；已有路径的符号链接会解析后检查。`serve` 创建缺失的 data/logs，但不创建配置或修改权限。`check` 不打开数据库、不创建日志或运行目录，通过也不代表目录可写。存储操作预算必须小于 `limits.shutdown_ms`，记录上限必须大于生成并发；完整范围见[设计契约](../design.md#服务配置与命令)。

`config.js` 示例：

```js
const upstreamKey = limen.env("UPSTREAM_API_KEY");
if (!upstreamKey) throw new Error("UPSTREAM_API_KEY is required");

export default {
  providers: {
    primary: {
      endpoint: "https://api.example.com/v1/chat/completions",
      auth: { bearer: upstreamKey },
      defaults: { temperature: 0.7 },
    },
  },
  models: {
    chat: {
      provider: "primary",
      upstream: "your-upstream-model",
      defaults: { temperature: 0.4 },
    },
  },
};
```

将 endpoint 和 upstream 改为实际服务值。endpoint 是完整 Chat Completions URL，Limen 不追加路径；只接受绝对 HTTP(S) URL，不接受 userinfo 或 fragment。provider 与 model 都需至少一项，model 的 provider 必须存在。

启动前先验证，再运行服务：

```sh
cargo run --locked -- check --config local/config/server.toml
cargo run --locked -- serve --config local/config/server.toml
```

`check` 会执行顶层 JS、导入模块并验证配置，不调用请求 hook、不监听端口、不请求上游。服务只在启动时加载配置；修改后需重启。

## 环境变量

`limen.env(name)` 是脚本唯一的宿主命名空间入口。变量不存在时返回 `undefined`，存在但为空时返回 `""`；`??` 只处理前者。名称必须非空且不含 `=` 或 NUL，名称与值受字节限制，非 Unicode 值会抛错。

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

权威字段与校验入口：[`types/limen.d.ts`](../../types/limen.d.ts)、[`modules/config/src/server.rs`](../../modules/config/src/server.rs)、[`modules/config/src/business.rs`](../../modules/config/src/business.rs)。
