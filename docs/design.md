# Limen 设计

本文是 Limen 的工作区设计基线，完整承接原始 `Limen-design.md` 的产品与技术约束，后续工作无需访问原始附件。
配置装载、有界 JS 调度、HTTP/1 网关、JSON/SSE 四路径及容器构建配置已经实现。
目标部署环境的流背压与容器运行验收仍须单独取得证据。
依赖解析以锁文件为准，升级时重新核对上游。
当前状态与剩余工作见[实施路线图](roadmap.md)，工程操作见[开发指引](development.md)。
现有功能的配置与使用方法见[功能与使用指南](features.md)。

“已确认”是产品边界，“建议默认”是实施起点，“待验证”必须以实现证据关闭。本文中的示例和验证清单是项目资料，
不代表已经执行，也不自行授权发布、真实上游调用或外部操作。设计变更维护在本文件中，不再依赖附件副本。

## 1. 目标与状态

### 已确认

- 名称为 **Limen**，使用 Rust 实现，服务个人和内网使用场景。
- 只提供 OpenAI 风格的 Chat Completions 接口，以及明确配置的模型列表；不做 UI。
- 配置 provider，包括上游地址、认证和请求处理；每个对外模型显式指向某个 provider
  的某个上游模型。
- provider 与 model 都可定义请求头、请求体等处理，执行顺序必须直观、可控。
- 基础服务配置与同目录 `config.js` 分工；JavaScript 为已选定的配置语言。
- 配置脚本支持本地 ESM 导入，模块在启动时求值，不按请求重新加载配置文件。
- 首版即支持异步 hook，为未来 I/O 留好运行时基础；当前不需要网络或文件 API。
- 主要部署目标是 Linux Docker，不以原生 Windows 发布为首版目标。

### 建议默认

- 基础配置使用 `server.yaml`；业务配置由 `config.js` 默认导出。
- 函数直接表达条件和动态计算，不设计表达式字符串、字段后缀语言或另一套脚本 DSL。
- provider/model 各自拥有有序 `pipeline`；同一阶段先 provider 后 model，各数组内部正序执行。
- hook 使用“修改当前 draft，完成值为 `undefined`”的单一契约，支持 `void | Promise<void>`。
- 首版不提供网络、进程、任意文件读写或 Node.js API；支持异步函数本身不等于开放这些宿主能力。
- 不做自动重试、故障转移、热重载和上游模型自动发现。
- 对外响应默认使用模型别名，允许显式响应 hook 改写；诊断信息另存真实路由和最终上游模型名。

### 待验证

上述异步与平台选择已经确定，运行时与平台能力仍需实施验证。

- **已有本机证据：**Rust 与 QuickJS 引用关闭顺序、Promise 调度与取消、执行/内存预算和数字边界已通过隔离实验；已知限制见第 9 节。
- **已有运行证据：**Rust xtask 的 17 个 VM 与 12 个本地合成 HTTP 实验在 Windows 和 Linux CI 通过，
  覆盖参数覆写、JSON/SSE 管线、认证隔离、单次发送、超时、客户端断开、启动就绪发布、DONE 分块一致性、
  OOM 保守退役边界、Windows Ctrl-C 与 Linux SIGTERM 关闭。
- **已有 CI 证据：**Linux CI 与 Docker 镜像构建已跑通。
- **实施时验证：**目标部署环境的流背压、慢客户端与关闭传播。
- **实施时验证：**Linux Docker 中的启动、配置挂载、端口绑定与关闭行为。
- **实施时验证：**依赖最新稳定版本、MSRV、平台及 feature 兼容性；本文不把网页上的 `latest` 快照作为锁定版本。

## 2. 最小对外接口

- `POST /v1/chat/completions`：接受显式配置的模型别名。
- `GET /v1/models`：仅列出配置中的对外模型，不连接上游做发现。
- 客户端传入未配置的 `model` 时返回明确错误，不自动把名称转发到任意上游。
- `GET /v1/models` 与生成接口使用相同的客户端认证策略，不建立独立权限体系。
- 首版不提供 Responses、Embeddings、Images、音频接口或 provider 原生协议转换。

Chat Completions 请求里的未知扩展字段默认保留。能够传递某个字段，不代表 Limen
能理解、生成或跨传输形态转换它。

## 3. 两层配置

### 3.1 `server.yaml`：进程与资源边界

基础配置只负责：

- 监听地址和客户端访问认证。
- 请求体大小、并发/排队上限、超时与关闭行为。
- JS 启动、hook 等待、请求整体、VM 执行及内存预算。
- 日志级别、格式及输出方式。

它不定义
provider、模型映射或业务变换。这样无需先运行用户脚本，就能确定日志、监听与脚本资源限制。

最小示例：

```yaml
listen: 127.0.0.1:8080
client_auth:
  bearer: "example-limen-client-token"
log:
  level: info
  format: text
```

示例令牌只是占位文本，不是真实凭据。实际值可以直接保存在本地配置文件中，不强制通过环境变量提供。

建议默认仅监听 `127.0.0.1`。需要局域网访问时显式修改监听地址，并配置客户端
bearer
token；无认证模式需显式写明。首版不做用户数据库、多租户、密钥分组或额度管理。

资源限制由 Rust 强制执行，业务 hook 不能取消。以下是当前 schema；所有字段可在 `limits` 中覆盖，未知字段报错。
默认值是面向个人使用的保守初值，用于给内存、队列和执行建立上界，不是测量所得的容量或吞吐保证。
HTTP 路径接入正文、并发、deadline、hook 和清理限制；流路径另外执行单事件、累计展开与聚合预算。

| 字段                   | 默认值   | 允许范围           | 用途                                   |
| ---------------------- | -------- | ------------------ | -------------------------------------- |
| `startup_ms`           | 5000     | 1–300000           | 模块求值、提取与静态校验的总等待预算   |
| `hook_ms`              | 5000     | 1–300000           | 单 hook 排队到结果转换的 wall deadline |
| `request_ms`           | 120000   | 1–3600000          | 请求总预算                             |
| `execution_ms`         | 100      | 1–10000            | 每次进入 JS 的统一执行时间上限         |
| `vm_work_ms`           | 1000     | 1–60000            | VM 持续可运行 job 的总工作上限         |
| `cleanup_ms`           | 100      | 1–10000            | 终态观察预算                           |
| `shutdown_ms`          | 5000     | 1–300000           | 应用等待 runtime 关闭的上限            |
| `memory_bytes`         | 67108864 | 1048576–1073741824 | QuickJS 分配预算                       |
| `stack_bytes`          | 524288   | 65536–8388608      | QuickJS 栈预算                         |
| `concurrency`          | 32       | 1–4096             | HTTP 在途请求上限                      |
| `queue_capacity`       | 64       | 1–65536            | JS 命令队列容量                        |
| `max_pending`          | 128      | 1–65536            | 未完成 hook 上限                       |
| `job_batch`            | 32       | 1–1024             | 每轮最多执行的微任务数                 |
| `max_body_bytes`       | 4194304  | 1–268435456        | 请求正文上限                           |
| `max_event_bytes`      | 1048576  | 1–67108864         | 单 SSE 事件上限                        |
| `max_output_events`    | 64       | 1–4096             | 单事件管线累计展开数量上限             |
| `max_output_bytes`     | 4194304  | 1–268435456        | JSON 桥接及事件展开输出上限            |
| `max_aggregate_bytes`  | 16777216 | 1–1073741824       | SSE 聚合字节上限                       |
| `max_aggregate_events` | 100000   | 1–1000000          | SSE 聚合事件数上限                     |
| `max_modules`          | 128      | 1–4096             | ESM 模块数上限，包含入口               |
| `max_source_bytes`     | 4194304  | 1–67108864         | ESM 模块图总源码字节上限               |
| `max_json_depth`       | 64       | 1–128              | JS/JSON 对象嵌套上限                   |
| `max_json_nodes`       | 100000   | 1–1000000          | JS/JSON 值节点上限                     |

基础 YAML 文件另外固定限制为 1 MiB、单文档和 16 层嵌套，拒绝重复键、merge key、anchor 和 alias。
认证必须明确写成 `{bearer: "..."}` 或 `{none: true}`，不能同时提供两者；令牌必须符合 bearer token 字符集。
`listen` 为非零端口的 IP socket 地址。`log.level` 接受 `error/warn/info/debug/trace`，
`log.format` 接受 `text/json`，`log.output` 当前仅接受 `stderr`，默认分别为 `info/text/stderr`。

### 3.2 `config.js`：路由与变换

默认读取 `server.yaml` 所在目录中的
`config.js`。路径不依赖服务进程的当前工作目录。

建议默认导出以下业务入口：

- `onRequest`：可选的全局、路由前 hook。
- `onError` / `onFinish`：可选的全局只读观察 hook。
- `providers`：按名称索引的上游连接定义。
- `models`：按对外模型别名索引的模型定义。

有序 `pipeline` 分别位于 provider 与 model 内部，hook
函数直接保存在步骤中。不另建字符串 hook 名称注册表。

下面是一个完整的小型业务配置。地址使用保留示例域名，不能直接请求真实服务。

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
          beforeSend({ body }) {
            const limit = body.max_completion_tokens;
            if (limit !== undefined) {
              body.max_completion_tokens = Math.min(limit, 4096);
            }
          },
        },
      ],
    },
  },
};
```

示例体现结构和顺序，不声称所有上游都支持 `temperature` 或
`max_completion_tokens`。具体 provider 的兼容性处理由其配置决定。

### 3.3 本地模块与秘密

`config.js` 和 `.mjs` 文件都按 ESM 处理，不要求
`package.json`。相对导入始终相对于导入者文件解析。

例如主配置可以正常使用：

```javascript
import { upstreamToken } from "./secrets.js";
import { compatibilityStep } from "./rules.js";
```

`secrets.js`
可直接导出本地字符串。文件应由用户自行保管，限制读取权限并排除版本控制；这属于部署建议，不把环境变量变成必经路径。

首版支持明确的本地 `.js`/`.mjs`
模块。相对路径以导入者所在目录为基准，并进行路径归一化，确保同一模块的身份和缓存一致；允许用户显式导入
`../shared/rules.js`
一类共享模块。配置按个人可信本地脚本处理，不额外建立配置目录沙箱。

首版不做 npm 下载、Node 模块解析、远程 URL 导入或运行时动态安装。

配置模块及其依赖图在服务启动时求值一次。它导出的函数会在请求过程中多次调用，因此
JS runtime 必须随服务存活。修改文件后重启服务才生效。

“启动时执行一次”不是“脚本没有副作用”，也不是“运行完便销毁 JS 引擎”。

### 3.4 环境变量接口

脚本通过全局只读接口 `limen.env(name)` 同步读取 Limen 进程继承的单个环境变量。
配置顶层、本地导入模块和 hook 均可调用；`limen` 与它的 `env` 属性不可替换或删除。
`limen` 命名空间发布前冻结，脚本不能添加属性或改变其原型。
接口由脚本库的统一宿主入口在用户模块求值前安装，JS 句柄和宿主回调始终留在拥有 VM 的线程。

- `name` 必须为非空字符串，不得包含 `=` 或 NUL；不隐式转换其他 JS 类型。
- 已设置返回 Unicode 字符串，保留空字符串与首尾空白；未设置返回 `undefined`。
  默认值使用 JS 的 `??`；必需密钥应由配置显式拒绝缺失或空值。
- 名称和值各自的 UTF-8 字节数均不得超过 `limits.max_output_bytes`；超出时抛出 `RangeError`。
  无效参数抛出 `TypeError`；操作系统值不是有效 Unicode 时抛出 `Error`，不替换字符或当作缺失。
- 每次调用按名称读取当前进程环境，不枚举、不预载全部变量、不提供写入或 `.env` 文件加载。
  顶层读取得到的 provider 字符串在启动时固定；外部修改环境不能更新已经运行的进程，变更后应重启。
- 错误使用固定文案，不回显变量名和值；沿用 JS 异常诊断的清理规则。
  接口按可信本地配置处理，可读取进程继承的任意变量，不是秘密隔离或权限沙箱。

此接口只增加环境变量读取能力，不开放其他进程、网络、任意文件或 Node.js API。
`server.yaml` 不执行环境变量插值；Docker 部署需显式将变量传入容器。

## 4. Provider 与模型的关系

### 4.1 Provider 字段

- `endpoint`：完整的 Chat Completions URL，包含实际路径。
- `auth`：可选的上游认证，例如 `{ bearer: "..." }`。
- `headers`：静态上游请求头。
- `defaults`：请求体字段的默认值。
- `pipeline`：该 provider 的有序处理步骤。

`endpoint` 不称为 `baseUrl`，也不自动补 `/v1`、`/chat/completions`
或尾部斜杠。原样使用经过校验的完整 URL，可避免 `/v1/v1`
一类错误，也兼容非标准部署路径。

启动时校验 `http`/`https` scheme、完整主机和非法 URL 形式；不允许把凭据放在 URL
userinfo 中。需要静态 query 参数时保留，但日志不输出原始 query。

### 4.2 Model 字段

- 对象键：向客户端展示和接受的模型别名。
- `provider`：已有 provider 的名称。
- `upstream`：实际发送给该上游的模型名。
- `defaults`：该模型的请求体默认值。
- `pipeline`：该模型的有序处理步骤。

模型通过引用复用 provider 的连接配置，不做 `extends`
链或隐式多重继承。需要静态配置不同 endpoint 或上游认证时定义另一个
provider；可用普通 JS 常量、函数或对象展开复用重复配置。

当前 schema 要求 1–1024 个 provider、1–4096 个 model；每条 pipeline 最多 256 步，步名在该数组内唯一。
provider 名、模型别名与步名为 1–128 字节的 ASCII 字母、数字或 `-._/`；`upstream` 为 1–1024 字节且不含控制字符。
endpoint 最多 8192 字节，显式使用 `http://` 或 `https://` 前缀（scheme 忽略大小写），拒绝空白、反斜杠、userinfo 和 fragment，
保留原始完整路径与 query。静态 `headers` 是字符串到字符串的对象，拒绝大小写重复的头名及 Rust 管理的传输头。
可选字段通过省略表达缺失，显式 `null` 不能代替 auth、hook 或其他配置记录；defaults 内的 JSON `null` 保留。
所有结构记录要求对象，不接受按字段位置排列的数组。bearer 最多 8192 字节，不允许空值或控制字符。

### 4.3 覆盖规则

请求体初始值采用明确的浅层优先级：

1. provider 的 `defaults`。
2. model 的 `defaults`。
3. 客户端当前请求体。
4. 用路由选中的 `upstream` 初始化 `body.model`。
5. 按顺序执行 provider 和 model 的 `beforeSend`。

“浅层”表示按顶层字段覆盖；数组和嵌套对象作为整体替换，不暗中递归合并。显式
`null` 不等于缺失，不触发默认值。

`defaults` 不允许提供 `model`、`messages` 或 `stream`
等控制/请求主体字段，避免把路由、会话和传输方式藏进默认值。需要改变这些字段时使用相应阶段的显式
hook。

管线不被模型覆盖掉，而是先运行 provider 管线，再运行 model
管线。同一字段的后续修改看到前一步的结果。

两层处理参考了
[Hikyaku 的 backend/route header 规则与 defaults/inject 配置](https://github.com/wentbackward/hikyaku/blob/main/docs/configuration.md)
所体现的复用思路；Limen 保持自己的 provider/model 顺序和普通 JavaScript
表达方式，不照搬其配置语言。

## 5. 认证、请求头与信任边界

客户端访问 Limen 的认证，与 Limen 访问 provider 的认证是两套独立凭据：

- `server.yaml` 中的 `client_auth` 验证来访客户端。
- provider 的 `auth` 或静态认证头用于访问上游。
- 客户端的 `Authorization`、Cookie 以及其他无关请求头不默认传给上游。

上游请求头从最小集合、provider 静态头和 provider 认证构建，再进入
`beforeSend`。可信本地 hook
可显式修改上游认证头，但不能改变已经完成的客户端认证结果。

静态 `auth.bearer` 与静态 `headers.Authorization`
同时出现时启动报错，避免两套值争夺优先级。需要其他认证格式时可以只使用明确的静态头，不另加一套相似的
`apiKey` 字段。

hook 中的 `headers` 是大小写不敏感、保留多值的受控 draft，提供
`getAll`、`append`、`set` 和 `delete`。`set` 替换该头的全部现有值，`append`
增加一项，`getAll`
返回值列表；不通过任意逗号拼接破坏多值语义，也不假装是完整浏览器 `Headers`
API。Rust
负责维护头部合法性、`Host`、`Content-Length`、`Transfer-Encoding`、`Connection`、`Content-Encoding`
以及最终 `Content-Type`。hook
写入这些保留的传输字段时明确报错，不能静默接受后又覆盖。

响应转发默认只保留适当的上游头；认证、Cookie、内部连接信息及不适用于转换后 body
的长度/编码信息不盲目复制。`afterReceive`
修改的是待发送的下游 status/header draft，不能倒过来篡改上游解析事实。

## 6. Pipeline 与 hook 契约

### 6.1 一个步骤对象

每个步骤有可诊断的
`name`，以及一个或多个阶段函数。没有条件/表达式字符串；条件直接写成普通
JavaScript。

```javascript
const removeStore = {
  name: "remove-store",
  beforeSend({ body }) {
    delete body.store;
  },
};
```

建议阶段如下：

| 阶段              | 位置                           | 可修改内容                        |
| ----------------- | ------------------------------ | --------------------------------- |
| `onRequest`       | 全局，客户端认证之后、路由之前 | 当前入站请求 body                 |
| `beforeSend`      | provider → model               | 最终上游请求 URL、body 和 headers |
| `afterReceive`    | provider → model               | 待发送下游 status 和 headers      |
| `onResponse`      | provider → model               | 一次完整响应 JSON body            |
| `onUpstreamEvent` | provider → model               | 解析后的上游 SSE JSON 事件        |
| `onStreamEvent`   | provider → model               | 待发送下游 SSE JSON 事件          |
| `onError`         | 全局，只读观察                 | 错误信息和请求元数据，不修改响应  |
| `onFinish`        | 全局，只读观察                 | 最终结果与统计，不修改响应        |

每个阶段都按 provider 数组正序、model 数组正序执行。响应阶段不会因为类似
middleware 的“回程”而倒序。

上下文分成当前可变 draft 与只读元数据。元数据至少包括请求
ID、当前阶段、对外别名、选中的
provider、配置上游模型名及冻结的客户端传输意图；不是全部配置对象或全服务可变状态。

`ctx.state` 是当前请求独立的可变 JSON
状态，默认空对象，在该请求的各阶段与流事件之间保留；不同请求绝不共享。`onError`
/ `onFinish` 只能读取其最终视图。需要步骤局部状态时可按步骤名放到 `state`
子字段中。

当前上下文固定 `ctx.meta` 与 `ctx.state` 的引用：meta 及其嵌套内容只读，state 可原地修改，
观察阶段则整个 context 深度只读。`ctx.body` 可修改或替换为合法 JSON 对象；URL、status 和 headers
仅在相应阶段开放修改，headers 必须通过受控方法操作，不能替换引用。
meta 包含 requestId、phase、model、provider、configuredUpstreamModel、clientStream、原始 upstreamStatus
和解析类别 upstreamContentType（json/sse/other）；路由前尚未确定的字段为 null。
观察上下文另提供 outcome、error（失败时）与 stats，均为只读快照。

把请求准备、实际发送与响应处理区分为明确阶段，参考了
[agentgateway 的 policy 分阶段结构](https://github.com/agentgateway/agentgateway/blob/e523afba89e667c20fedaf9d00ae2ab312d6f9fb/crates/agentgateway/src/store/policy.rs)；本设计不因此引入其完整策略系统。

### 6.2 单一的修改与返回规则

普通 hook 原地修改当前 draft，契约为 `void | Promise<void>`：同步函数直接返回
`undefined`；异步函数返回 Promise，成功完成值必须是
`undefined`。不同时支持“改入参自动提交”和“返回对象覆盖入参”两套含混规则。

- 直接返回普通对象、`null` 或数组，或者 Promise 最终解析为这些值：契约错误。
- Promise rejection 与同步异常都让当前 hook 失败。仅识别真正的 JS
  Promise，不扩展自定义 thenable 协议。
- 抛异常、超时、输出不能转换回允许的 JSON：当前请求失败。
- 前一步的结果成为后一步的输入；没有隐藏回滚到原始请求。
- 服务仅在当前 hook 成功完成并通过输出校验后，提交其
  draft、请求状态及事件输出；异步等待期间的修改不提前交给后一步。调用失败时不继续把半完成结果发往上游或客户端。
- 这种提交边界不回滚 JS 闭包或全局变量的修改；不要把它宣传为脚本事务。

`beforeSend` 后的 `body.model` 与 `body.stream`
是最终上游请求值，不在发送前又被路由层偷偷覆写。它们不会触发第二次 provider
路由。

`beforeSend` 还可修改 `ctx.url`，初始值为 provider 的完整 `endpoint`。Rust
校验最终 URL 后原样发送；修改 URL 不重新选择
provider，也不在后续阶段恢复原地址。日志记录实际目标的无秘密形式；认证仍以最终
outbound headers 为准，不再次静默注入。

同一请求严格顺序等待：一个步骤完成后才进入下一个步骤，provider 阶段完成后才进入
model
阶段；当前流事件处理完才处理同一流的后续事件。不同请求可以在异步等待时交错推进，但不会并行执行同一个
JS runtime 中的字节码。

原有同步配置无需改写。需要异步逻辑时可直接声明
`async beforeSend(ctx) { ... }`；只有所返回的 Promise
完成后才提交结果。所有衍生异步工作都必须包含在该 Promise
的完成链中，不支持脱离请求生命周期的后台任务。

### 6.3 流事件的 drop 与多事件输出

流 hook 默认修改当前事件并输出一次。丢弃和拆分必须显式表达，不能把漏写
`return`、`undefined` 或 `null` 当作控制流。

流阶段使用 `ctx.event` 作为可修改或替换的 JSON 事件对象，仅在流事件上下文提供输出控制方法：`ctx.replaceEvents(events)`。

- 未调用时：输出修改后的当前事件一次。
- `ctx.replaceEvents([])`：明确丢弃当前事件。
- `ctx.replaceEvents([eventA, eventB])`：按数组顺序替换为多个事件。

同一次调用最多设置一次替代输出，重复设置时报错。替代事件继续交给本阶段后续步骤，不重新从第一步执行。
每个源事件累计所有成功 hook 的输出数量与 JSON 字节，包括默认的一入一出，分别受
`max_output_events` 与 `max_output_bytes` 限制；每个输出还须满足单事件和 JSON 结构限制。
VM 在提交时再次检查替代数组数量，防止异步等待期间增长绕过上限。

流 hook 同样遵循 `void | Promise<void>`；异步完成值仍必须为
`undefined`。`ctx.replaceEvents` 的结果仅在整个 hook
成功完成后提交，不再同时提供 `drop`、`emit` 或返回事件数组等重叠语义。

Rust 控制 `[DONE]`、错误控制帧、heartbeat、SSE framing
和连接关闭，这些不在可替换的 JSON 事件集合内。协议关键事件和结束条件也要在 hook
后校验。

### 6.4 Helpers 不形成第二套语言

常规字段增改删使用 JavaScript 自身语法。复杂重复逻辑用本地 ESM 函数复用。

只有能减少真实重复、保持一致语义的操作才值得提供 helper，例如：

- 大小写不敏感的 header 修改。
- JSON Pointer 上的严格字段操作。
- 生成一个相同步骤形状的配置工厂。

helper 是普通函数，直接操作当前 draft 或返回普通步骤对象；不再提供字符串
`expr`、字符串 `when`、后缀字段操作、独立模板语言或隐藏优先级。

如果引入 JSON Pointer helper，建议约束为：父路径必须存在；`set`
只新增/覆盖对象末级或替换已有数组下标；`remove`
缺失时无操作；默认值只填缺失而不替换 `null`。`move`/`copy`
源缺失报错且保持原子性；`append`
只向现有数组追加一个元素。不做通配符或自动创建中间结构。

普通 JS 对象操作保持 JavaScript 自身语义，严格 Pointer helper
的约束不偷偷套用到原生语法上。

## 7. 请求生命周期与传输选择

### 7.1 路由只发生一次

1. Rust 校验入口、客户端认证和请求基本形状。
2. 记录原始客户端 `stream` 意图，作为下游响应形态依据。
3. 调用并等待全局 `onRequest`；其修改后的 `body.model` 参与模型查找。
4. 选择模型和 provider，冻结路由身份。
5. 合并默认值，初始化上游模型名与上游请求头。
6. 顺序调用并等待 provider → model 的 `beforeSend`。
7. 校验最终上游请求并发送。
8. 按真实上游响应状态和媒体类型选择处理路径。

`onRequest` 修改 `stream` 不改变已经记录的下游传输意图。需要改变上游使用 JSON
还是 SSE，可在请求 hook 中显式设置 `body.stream`。

不存在“修改上游 model 后又选择另一个
provider”的隐式行为。未来如果需要动态后端选择，应作为新的明确能力设计，而不是复用一个含糊字段。

### 7.2 JSON 与 SSE 的四种组合

| 上游 | 下游 | 处理次序                                                       |
| ---- | ---- | -------------------------------------------------------------- |
| JSON | JSON | 完整 JSON → `onResponse` → 输出                                |
| JSON | SSE  | 完整 JSON → `onResponse` → 转换成事件 → `onStreamEvent` → 输出 |
| SSE  | JSON | `onUpstreamEvent` → 聚合 → `onResponse` → 输出                 |
| SSE  | SSE  | `onUpstreamEvent` → `onStreamEvent` → 输出                     |

SSE → SSE 不因为配置了 `onResponse` 就自动缓存整段流。`onResponse`
只在真正有完整响应对象的路径执行，每次请求最多一次。

`afterReceive` 修改待发送的下游 status/header draft。Rust 保留原始上游
status、content-type 等事实，用于错误判断和 parser 选择；改 draft
不会让失败响应进入成功路径，也不会使 JSON body 被当成 SSE
解析。建议允许显式映射失败响应的下游错误状态码，禁止把原始失败映射到 `2xx`
成功类别。当前契约固定为：原始 2xx 仅可映射到 2xx，且下游不得为无正文的 204/205；原始 3xx–5xx 仅可映射到 4xx/5xx。1xx 与其他非法状态拒绝，不由各 hook 自行定义错误分类。

JSON → SSE 只是兼容性分段，不会降低上游完整生成所需的首包延迟。

SSE 内部采用“解码 → 语义处理 → 编码”的职责分离，可参考
[agentgateway 的流转换实现](https://github.com/agentgateway/agentgateway/blob/e523afba89e667c20fedaf9d00ae2ab312d6f9fb/crates/llm/src/parse/transform.rs)。这与把任意网络字节块当作模型事件不同。

### 7.3 对外模型名

建议默认使用对外模型别名：

- 完整响应：在进入 `onResponse` 之前，把存在的标准 `model`
  字段规范化为对外别名。
- SSE：在进入 `onStreamEvent` 之前，对标准 chunk 的 `model`
  字段应用对外别名默认。
- 用户 hook 可在此后显式覆盖；输出端不再强制覆盖一次。

路由身份、配置上游模型名、`beforeSend`
后真正发出的模型名分别记录。日志不能因为响应模型别名改写而丢失最终上游值。

JSON → SSE 转换须保留 `onResponse`
已显式修改的响应模型名，而不是二次套用别名默认。默认值只在对应路径第一次形成对外响应表示时应用。

### 7.4 HTTP 适配

当前 HTTP/1 入口使用 Hyper 连接驱动与 Axum body/response 类型。两个 API 共用客户端认证与有界请求处理，
入口正文限制为 `max_body_bytes`；合并后的上游 body、上游 JSON 和最终 JSON 输出受 `max_output_bytes` 限制。
初始请求必须为对象，包含非空字符串 `model`、非空对象数组 `messages`；`stream` 只能为布尔值或省略。
消息内的扩展字段保留，具体消息协议兼容性由上游判断。当前只接受 JSON 媒体类型（忽略大小写及参数）。

客户端原始 `stream` 决定下游 JSON 或 SSE，hook 最终的 `body.stream` 原样发送上游。
解析路径由上游原始媒体类型决定，不根据请求猜测。上游 2xx 必须为完整 JSON 对象或合法 Chat SSE；
无正文状态、非法 JSON、过大正文与读取失败进入错误路径。
原始 3xx–5xx 默认映射为 502，`afterReceive` 可显式映射到 4xx/5xx；不读取或回显失败正文。
响应头默认仅保留 content-language、x-request-id 和 retry-after，并排除 Connection 指定的逐跳字段；
可信 hook 可显式增改普通下游头。最终 Content-Type 由 Rust 根据实际下游路径设置。

reqwest 客户端明确禁用重试、重定向和环境代理，使用 Rustls 与平台证书验证。
配置中的有限 origin 复用一个连接池，每个 origin 至多保留一个空闲连接，空闲超时为 30 秒。
hook 选择其他 origin 时使用不保留空闲连接的短期客户端；活动客户端受并发限制，避免长期积累任意目标的连接。

### 7.5 非文本内容与未知字段

建议将“同形态传递”和“跨形态转换”分开承诺：

- 请求中的多模态内容、工具定义及未知扩展字段默认保留，除非配置显式修改。
- 同为 JSON 或同为 SSE 时，不理解但合法的字段尽量保留，不做无理由删减。
- JSON ↔ SSE 首版转换覆盖文本、refusal、function tool calls、多 choice（按 index
  独立处理）、finish reason 和 usage；tool call 的 ID、函数名和 arguments
  分片都须正确归并或拆分。
- 不支持的跨形态结构返回明确错误，不静默丢字段或拼成假文本。

首版不把“能转发音频/图片相关字段”宣传成“已实现完整多模态跨形态转换”。字段结构以
[OpenAI Chat Completions 官方参考](https://developers.openai.com/api/reference/resources/chat)
为依据；这里的支持范围是 Limen 的建议契约，不代表上游全部能力。

## 8. 流、错误与取消

SSE 按事件而非 TCP 分块解析，支持跨分块 UTF-8、CRLF、LF、CR 与多行
`data:`。解析以
[WHATWG SSE 标准](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream)
为依据；模型 JSON、usage 和 `[DONE]` 属于上层 Chat Completions 协议。
输入预算覆盖归一化后的事件块，CRLF 的 LF 作为同一换行被忽略；解析缓冲另有同值上限。
输出数据帧单次有界编码，按包含 framing 的编码字节数限制；转换拆分的尺寸试探仍采用无输出缓冲的计数。
Rust 的固定错误控制帧使用独立的 1 KiB 上限，
避免较小的数据事件预算阻止发送约定错误对象。

- `[DONE]` 是协议结束标记，默认要求成功流明确结束；读到首个完整结束标记后立即停止上游读取与解析，
  不等待 socket EOF，也不再处理同一网络块的剩余字节。EOF 未收到结束标记按截断失败处理。
- `finish_reason` 不等于流已结束。其后可能还有 `choices: []` 的 usage
  chunk，不提前关流。
- 多 choice、tool call 的 index 与分片字段必须正确累积，不把不同 choice
  或调用混在一起。
- 流的 choice 完成状态最多跟踪 `max_json_nodes` 个不同 index；一个事件可包含多个 choice，
  不把聚合事件数限制误当成每事件的 choice 数限制。
- 聚合有最大字节数与事件数，不能因为客户端选择 JSON 就无限缓存上游流。
- SSE→JSON 聚合形成的完整对象也须满足 JSON 深度、节点与安全整数限制；逐事件校验不能替代累计结果校验。
- 下游断开时取消上游读取、待执行 hook
  和相关任务，尽快释放本地资源并向上游中止连接；这不能保证 provider
  已开始的生成或计费立即停止。
- 排队、慢客户端和 VM 执行都有背压，不创建无限任务或无界缓冲。

返回头尚未发送时，错误使用 `{"error":{"code":"...","message":"...","request_id":"..."}}`。
消息由 Rust 提供清理后的固定说明，不包含上游正文、JS 异常文本、token 或源码。
下表固定 HTTP JSON 与 SSE 流内错误共同的对外 code/status 契约。

| code                                    | HTTP 状态                                 |
| --------------------------------------- | ----------------------------------------- |
| `invalid_request` / `unknown_model`     | 400                                       |
| `unauthorized`                          | 401                                       |
| `payload_too_large`                     | 413                                       |
| `busy`                                  | 429                                       |
| `cancelled`                             | 499（内部归类；客户端已断开时不尝试发送） |
| `hook_failed` / `internal_error`        | 500                                       |
| `upstream_failed` / `upstream_protocol` | 502                                       |
| `unavailable`                           | 503                                       |
| `deadline_exceeded`                     | 504                                       |

内部错误另外保留阶段、调用/请求 ID、清理是否执行及退役原因，不直接序列化给客户端。
开始发送 SSE 后无法更改 HTTP 状态；发送一个 `event: error` 事件，其 `data` 为相同错误对象 JSON，随后关闭连接，
不追加 `[DONE]`。SSE 媒体类型匹配忽略大小写并容忍参数，仅接受 `text/event-stream`；
JSON 与 SSE 跨形态转换遇到未支持结构必须拒绝，不能静默丢失字段。

首版没有自动重试，并关闭自动上游重定向。网络错误、上游失败或跳转不隐式重新提交生成请求，以免产生重复费用、重复输出或跨目标传递凭据。

hook
错误默认让当前请求失败，不悄悄跳过规则后继续发送。日志保留阶段、步骤名、请求 ID
和经过清理的错误位置；对外不泄露脚本内容或秘密。

错误响应格式和 HTTP/SSE 错误映射由核心统一处理。`afterReceive`
可修改允许范围内的下游
status/headers，但不能将上游失败或脚本异常改写成成功。自定义错误体转换可作为后续显式扩展，不与成功响应
hook 混用。

建议另提供只读 `onError` / `onFinish` 观察
hook，不修改响应或恢复失败。成功、失败与取消都进入一次终态观察流程；观察 hook
自己失败仅记录日志，不递归触发错误观察或改变请求最终状态。

观察 hook 同样支持
`void | Promise<void>`，由调度器追踪并使用独立、有限的清理预算，不能无限拖住请求结束或服务关闭。Rust
侧只记录一次最终 outcome；VM 正常时只调度一次 `onFinish`。若 VM
已退役无法调用观察函数，则记录未执行原因，不假称 JS 回调已经成功。

JSON 在响应结果固定后立即交付 HTTP 层；SSE 在首次数据帧时交付头部，用容量为 1 的正文队列传递后续帧，
不因 `onResponse` 缓存整段 SSE。生产任务等待 HTTP body 消费结束或取消，再进入一次终态观察。
错误写出、错误观察（失败或取消时）与结束观察共享从终态开始的 `cleanup_ms` 绝对预算，耗尽后记录未执行原因。
慢下游的错误帧只在此预算内尽力交付；DONE 已入队后不追加错误帧。观察失败不会改变已固定结果。
观察任务保留在请求 JoinSet 中并占用请求并发许可，服务关闭时等待它们；容量拒绝的观察在有界连接任务内等待，
不另外创建无界任务。JSON 终态表示管线已生成结果，输出字节数表示生成的 JSON 大小；
SSE 字节数累计已入队数据帧，消费完成确认只表示 HTTP 层读取了 body，不承诺客户端已经收到全部字节。

HTTP 连接任务上限为 `concurrency + queue_capacity`，不再创建额外等待队列；请求许可不足时返回 429。
JS 命令队列与 pending 上限独立执行。连接固定限制 100 个头和 32 KiB 头部/缓冲，并有读取头部的超时。
每次进入服务时建立 `request_ms + cleanup_ms` 绝对连接期限，覆盖正文读取、管线与 JSON/SSE 写出；
随后进入空闲 keepalive 时继续保留该期限，下个请求进入服务时重新计时。超时关闭连接并记录传输失败。
客户端断开会取消本地请求任务的推进和上游读取，但请求任务保留到有限观察结束。
该连接期限已有编译和 deadline/关闭接线证据；本机 Windows 写入由系统排队，未获得可靠的阻塞写实验，
Linux 慢客户端和流背压仍需单独验证。

## 9. JavaScript 运行时

### 9.1 共享 VM 与可推进的异步调度

Rust HTTP 服务使用 Tokio；独立 OS 线程运行 current-thread Tokio
executor，并拥有一个 rquickjs `AsyncRuntime`、`AsyncContext` 及持久化 hook
表。启用 `futures`、`loader` 所需能力，不依赖实验性 `parallel` feature。HTTP
侧只传 hook ID、请求/调用 ID 和 owned 数据，不跨线程传 JS 句柄。
入队前校验 JSON context 的结构与数字边界，并在字节预算内编码一次；命令包含该 owned JSON 字节、
绝对 `Instant` deadline 与取消句柄，VM 直接解析这些字节。一次性结果通道关联原调用，
只交付通过校验的 owned JSON。取消调用 future 会关闭结果通道并唤醒 actor，从而撤销提交资格。
编码入队后不再保留原始输入 JSON 树；VM draft 建立后释放输入字节，pending 仅保留 deadline、取消信号、
结果通道和 VM 内的 draft/Promise 引用。取消与 deadline 在入队及提交边界仍分别检查。

一个共享 VM 保留“每次进程启动求值一次配置”的语义。首版不建立多 VM
池，也不为每个请求重新执行模块。不同请求可以在等待 Promise 时交错推进，但 JS
字节码仍串行运行。

入口队列与等待中的 hook 数分别有界。调度器必须同时处理调用、完成、取消与
deadline，不能在收件 actor 中直接 `await` 某个可能永不完成的
Promise。当前采用显式、有限批次的 job pump：

1. 接收有界调用命令，创建该调用独立的 draft 和状态快照；在短
   `AsyncContext::with` 中调用函数。
2. 返回 `undefined` 时进入校验/提交；返回原生 Promise 时持久保存 Promise
   与当前调用对象，登记 pending 项后立刻继续事件循环。
3. 在新命令、取消、deadline 和 Promise 处理之间交替推进。每轮通过
   `AsyncRuntime::execute_pending_job` 执行有限数量的
   job，达到批次或时间片边界便让出执行权。
4. 批次间读取待处理 Promise 的结果；`Promise::result` 只读取状态，不负责驱动
   job。已完成调用按调用 ID 返回结果，再由请求流水线推进下一步。
5. 存在 pending Promise 但没有可执行 job 时，等待新命令、未来宿主 I/O
   完成通知或最近 deadline，不忙轮询。

不以常驻 `idle()` / `drive()` 的全队列 drain，或
`async_with(... promise.await)`，替代上述可控事件循环。否则无限微任务或悬而未决的
Promise 可能阻碍接收取消、超时与其他请求。

未来宿主 I/O 应以 Rust future / 完成通知唤醒该调度器，在等待期间释放 VM
访问权，再把完成结果交回原调用。首版只建立可承接该扩展的调度边界，不提前实现宿主
I/O。以后不能只轮询 `is_job_pending`：内部 Rust future
存在不代表立即可运行，其完成必须能唤醒 job pump。

该结构依据
[rquickjs AsyncRuntime 单步 job API](https://docs.rs/rquickjs/latest/rquickjs/struct.AsyncRuntime.html#method.execute_pending_job)、[AsyncContext 短调用 API](https://docs.rs/rquickjs/latest/rquickjs/struct.AsyncContext.html#method.with)
和
[Promise 状态 API](https://docs.rs/rquickjs/latest/rquickjs/struct.Promise.html#method.result)。
本机调度、唤醒与取消的行为证据见第 9.4 节，不把启用了 async feature 当作调度或隔离保证。

配置中的全局变量和闭包会跨请求存活，并可能在 `await`
边界发生逻辑交错。单线程不等于请求隔离；请求数据只放在独立 draft / `ctx.state`
中，不缓存到全局对象或脱离返回 Promise 的后台任务中。

### 9.2 导出提取与生命周期

加载配置时先识别、保存函数引用，再把静态 provider/model 数据转换为 Rust
结构。不能对整个默认导出 `JSON.stringify` 后再反序列化，因为函数会丢失。

同步和 async 函数使用同一 hook
注册机制；不按函数声明形式猜结果，运行时检查其实际返回值。异步调用持有
Promise、draft 和状态引用，直到成功、失败或取消清理结束；函数和 Promise
的持久引用都不能超过所属 runtime 生命周期。

官方 API 中，模块声明、求值与读取导出是不同步骤。`Module::declare(...).eval()`
的完成结果需要在启动预算内驱动并等待，再读取模块的 `default`
导出；允许模块求值中的 top-level
await，但默认导出本身仍须为配置对象。不能误以为任意求值 API 都直接返回模块对象。

持久化函数引用不能活得比所属 runtime 更久。关闭服务时先停止新请求并处理 pending
调用/宿主 future 的取消，清理持久化 Promise 与请求对象，再释放函数引用、context
和 runtime。

运行时先发布 `Ready`，再通知调用方启动成功，避免成功返回后仍可观察到 `Starting`。

### 9.3 ESM 与宿主能力

rquickjs 提供 resolver/loader 接口；当前适配器增加路径归一化、`.mjs`、普通文件检查、模块数量与总源码预算，
通过 `Module::declare` 交给 QuickJS 解析和缓存。内建文件加载器不能承担这些资源边界，因此仅替换文件解析与读取层，
不重写 JavaScript parser，也不实现 Node 兼容层。

QuickJS 是 JavaScript 引擎，并不自动提供
Node.js、`fetch`、定时器或浏览器标准库。首版不承诺这些能力；配置复用依靠本地模块和普通
JS。宿主仅另外提供第 3.4 节的只读 `limen.env(name)` 接口。

脚本可访问的宿主能力由 `host::install` 集中注册：创建命名空间、显式注册函数、冻结对象，
最后以不可替换、不可删除的全局 `limen` 绑定发布。该入口在 VM/context 创建并设置预算后、
用户模块求值前调用一次。各能力模块只构造函数及维护自身参数、预算和错误语义，不自行写入全局对象。
当前 `host/env` 实现环境变量读取，无需动态注册表或插件机制。

`limen` 保存 VM 级宿主能力；每次 hook 的 `ctx` 继续承载请求、draft 与 state。
JSON 桥接的代理检查和 header 校验通过工厂参数私有传入，不加入公开 `limen` 命名空间。

### 9.4 超时、取消与共享 VM 边界

区分“某次调用不再等待”和“JS 执行已被中止”，不把取消 Rust future
或丢弃响应通道等同于取消 JavaScript Promise。

- 每次调用拥有独立 wall
  deadline，覆盖排队、异步等待、执行和结果转换；请求仍有整体截止时间。
- 普通超时或客户端取消时，移除对应调用表项、撤销结果提交资格，并释放宿主持有的
  Promise/draft
  引用。该调用不提交晚到结果，也不进入下一步；其他请求继续推进，普通等待超时不默认重启进程。
- 释放宿主引用不保证移除 JS 自己持有的引用或
  continuation，不回滚已经发生或迟到的全局/闭包修改。配置代码必须遵守请求独立状态与不启动游离后台工作的契约。
- 未来加入宿主 I/O 时，调用 ID 和取消信号必须贯穿其任务；取消对应 Rust future
  或忽略晚到完成。仍不能据此保证远端操作或计费已经停止。

CPU 预算与 wall deadline 分开：引擎中断处理器限制每次实际 JS
执行；事件循环限制微任务批次并检查 VM 级连续工作预算。仅在外层套
`tokio::time::timeout` 无法打断同步死循环。

共享 runtime 的全局 job
队列不能简单归属当前收件请求，因此不能把某请求的超时信号直接用来打断此刻可能属于另一请求的
JS job。每次 JS 执行采用统一预算，各调用 wall deadline
则由调度器独立处理；不宣称首版已实现精确的每请求 JS CPU 计费或隔离。

同步执行被中断、传播到宿主的内存耗尽错误，或持续微任务工作超过 VM 级失控阈值时，将共享 VM
标记为不可继续使用：停止接受新调用、失败依赖它的未完成调用、记录原因并退出进程，交由
Docker
的部署重启策略恢复。其他在途请求可能因此失败；不得自动重新提交它们的上游生成请求。

不能可靠检测任意“全局污染”，也不以看似正常的 Promise 完成来证明 runtime 已恢复。
当前使用 QuickJS 内建 allocator，不启用 `rust-alloc`，`set_memory_limit` 在隔离实验中触发未捕获 OOM 并退役。
rquickjs 0.14 的安全 API 没有分配失败回调：如果 JS 自行捕获并消化 OOM，宿主不能可靠得知发生过分配失败，
因此不能承诺此时必然退役。内存上限仍然生效；配置脚本不得把捕获 OOM 后继续执行当作支持的恢复策略。
异常识别采用保守退役策略：抛出的值为 `null`，或非 Proxy 对象自身的数据属性 `message` 恰为
`"out of memory"` 时，也按 OOM 处理；普通脚本主动抛出相同值会触发误判并使共享 VM 退役。
异常分类器自身失败也按无法保证 VM 可继续使用处理。`memory-limit` 隔离实验已在 Windows 与 Linux CI
验证真实未捕获 OOM，以及同步异常、Promise rejection 中的上述误判边界。
改变此限制需要重新评估 allocator 接口与工作区默认拒绝 `unsafe` 的约束。

当前进程实验验证了同步/异步 void、悬挂 Promise 下的调用与取消推进、deadline 后迟到结果不提交、
同步中断、持续微任务预算、未捕获 OOM 和 pending 引用关闭路径。无 job 时通过通知、命令或最近 deadline 唤醒，
没有固定间隔轮询；上述实验已通过 Windows 与 Linux CI，这属于代码与行为证据，不是 CPU 占用或公平性吞吐测量。

### 9.5 JSON 边界

当前静态导出与 hook 输出只接受普通对象（含 null prototype）及普通数组；拒绝 accessor、Proxy、Symbol 键、
额外数组属性和非枚举数据，避免转换过程中执行 getter 或静默丢值。JSON 桥接按最坏转义长度保守计入字节预算，
可能在实际编码达到上限前拒绝；节点、深度和源码图也分别有界。

- 区分属性缺失、`undefined` 和 JSON `null`。删除字段用
  `delete`；把字段或数组元素赋为 `undefined`
  作为输出时明确报错，不静默变为缺失或 `null`。正常 hook 的返回值 `undefined`
  是独立的函数契约。
- 只接受允许的 JSON 值；拒绝循环对象、函数、Symbol、BigInt、NaN、Infinity
  等无法按契约表示的输出。
- JavaScript Number 不能无损承载任意 `i64/u64`。首版建议对实际进入 hook
  的输入/输出校验安全整数范围，超出 `±(2^53−1)` 的整数明确报错；不需要 hook
  的路径无需为了统一流程而进入 JS。原始值保留可后续单独设计。
  当前 HTTP JSON 路径统一采用该安全整数范围和深度/节点限制，即使没有 hook 也明确拒绝越界整数。
- 不声明“Rust → JS → Rust 对所有 JSON 数字完全无损”。
- Promise 是 hook 完成协议的一部分，不是 JSON 数据；先等待其完成值为
  `undefined`，再验证并转换 draft 和 `ctx.state`。检查完成值时先读取 JS `Value`
  并明确判断 `undefined`；不能依靠 Rust `()` 转换，因为该转换可能忽略实际值。

## 10. 校验、日志与运维

### 10.1 CLI

CLI 使用 clap 定义两个基本入口：

```text
limen serve --config ./server.yaml
limen check --config ./server.yaml
```

默认配置路径是 `server.yaml`，支持 `--config PATH` 和 `--config=PATH`，按原生文件系统路径解析。
顶层提供 `--help` / `-h` 和 `--version` / `-V`，两个子命令提供各自的帮助。
帮助与版本输出到 stdout 并返回 `0`；参数错误输出到 stderr 并返回 `2`，不回显任意参数值。

`check` 与 `serve`
共用配置加载、模块求值、导出提取和结构校验逻辑，但不监听端口、不调用
provider，也不执行请求阶段 hook。

`check` 会执行配置 JS
的顶层代码及导入模块；它不是无副作用的安全扫描器。禁止宿主网络/写文件能力可以减少副作用，但不能把本地代码执行说成纯文本校验。

建议检查：

- YAML 类型、重复键、未知字段和资源配置范围。
- 本地模块解析、语法、启动预算、默认导出形状。
- provider/model 名称、模型引用、endpoint 和 header 合法性。
- 管线结构、阶段名、步骤名及函数类型。
- 静态认证冲突、禁止的 defaults 控制字段。
- JSON
  数据可表示性、模块求值是否在启动预算内完成，以及默认导出是否仍为配置对象。允许异步
  hook 不代表允许用 Promise 代替整个默认配置对象。

静态 `check` 无法证明任意 hook
在所有输入下正确。可用最小合成输入对纯变换逻辑做单元测试，但这与配置结构检查是不同验证层。

### 10.2 日志

使用 `tracing`，默认记录足以定位问题的结构化元数据：

- 启动配置位置、配置加载成功、provider/model 数量。
- 请求 ID、对外模型、provider、实际目标、最终上游模型、输入/输出传输方式。
- 上游状态、首包/总耗时、字节数、完成/取消/截断原因。
- 失败阶段、步骤名、预算耗尽和有限的错误位置。

默认不记录完整请求体、响应体、消息文本、原始头、令牌、Cookie 或 URL
query。错误、调试日志和 JS 日志同样需要处理秘密，不能只在 INFO 级别遮蔽。
当前目标日志只保留 scheme、host 和 port，连可能携密的 URL path 也省略；发送时仍使用完整 endpoint。
请求 ID 由 Rust 分配，并通过 `x-limen-request-id` 及统一错误对象返回。
观察 stats 的 upstreamHeadersMs 表示从请求开始到收到上游头的时间。

YAML 诊断提供清理后的 schema 字段路径以及解析器确实提供的行列；缺少位置时不伪造坐标。
任意用户键名以 `*` 代替，避免把携密键名带入错误。求值生成的 JS 对象只报告静态 schema 路径，
不假定对象属性有可靠源码映射。JS 异常目前输出阶段/固定分类，不回显 message、stack 或源码；
不会为了位置而输出可能携密的异常文本。应用提供逻辑配置文件类别，不打印整份配置。

Rust 内部保留配置、I/O、解析器和引擎错误的原始类型与 `source()` 链，调用方按稳定分类处理；
原始来源可能包含秘密，不直接用于日志或 CLI。`diagnostic()` 显式遍历有界原因链，只输出操作、
错误分类和安全坐标，未知来源使用固定分类；不把清理后的文本重建为待传播的错误。
JS 异常对象和句柄仍只留在 VM 线程，跨线程仅保留 owned 引擎错误及分类，不提取异常 message/stack。
同步模块图加载可保留 loader 的 I/O 来源；动态 import 可能被 JS 捕获或与其他 Promise 交错，
因此不将共享 loader 诊断强行归给异步求值或某个 hook，其失败保留引擎提供的 Exception 分类。
配置校验与关闭同时失败时保留主失败和独立的清理失败，清理失败不能覆盖校验结果。

应用统一初始化 subscriber，并在处理边界记录一次最终失败。预期配置拒绝采用 `warn`，
预算、线程、退役或关闭等操作失败采用 `error`；日志尚未初始化或对应级别被过滤时，
CLI 仍向 stderr 输出安全诊断。启动、关闭和 hook 操作使用稳定 span 名和显式元数据，
异步执行按 future 传播 span；panic hook 只报告故障位置，不输出任意 payload。

首版不增加管理后台、远程修改配置接口或配置监控。修改配置后先
`check`，再重启进程。

### 10.3 Linux Docker 部署

首版优先提供 Linux 容器运行方式。镜像构建阶段准备 Rust 与 QuickJS 所需的 C
工具链，运行阶段只保留服务及所需运行依赖；具体基础镜像和 CPU
架构在实施时按实际部署目标验证，不顺带承诺原生 Windows 或多架构发布。

配置文件及本地 ESM
依赖通过目录挂载提供，不将真实令牌写入镜像层。共享模块的相对路径要在容器内保持可解析，挂载
`config.js` 时不能遗漏它引用的文件。

Compose 默认使用已发布的 `ghcr.io/rhnu/limen:latest`，将宿主 `data/` 挂载到容器工作目录 `/data`。
检查与启动均使用当前目录的 `server.yaml`，业务配置位于 `/data/config.js`；Docker 示例可独立复制到 `data/`。
配置文件由部署者准备，应用不自动生成。镜像 entrypoint 以 root 修正 `data/` 的组与组读取、目录访问权限，
保留宿主文件属主，再通过 `setpriv` 清空附加组、继承和 ambient capabilities，禁止提权，以 `10001:10001` 执行应用。
权限处理或降权失败时不启动应用；显式指定非 root 用户时跳过修复，由部署者准备读取权限。
`data/` 为支持权限修改而读写挂载，容器根文件系统保持只读；不承诺配置挂载只读。
入口不绕过宿主文件系统、用户命名空间或安全标签限制。

Docker 部署中，容器内通常需显式监听 `0.0.0.0`
才能经发布端口访问；宿主机端口绑定决定对本机还是局域网开放。裸进程的默认
`127.0.0.1` 不因此改变，也不把容器内部监听所有接口等同于向公网暴露。

收到 `SIGTERM` 时停止接收新请求，取消或在限定时间内排空在途请求、hook
和观察任务，然后释放 JS
资源。容器终止宽限时间应覆盖这一过程；超出关闭预算时明确结束，不无限等待
Promise。

## 11. Rust 依赖候选

依赖职责尽量直接，不为了统一包装而完整复制成熟库 API。

| 职责       | 候选                        | 设计阶段判断                                                                |
| ---------- | --------------------------- | --------------------------------------------------------------------------- |
| 异步运行时 | Tokio                       | 成熟生态；承担服务并发、取消、通道与超时                                    |
| HTTP 入口  | Hyper、Axum                 | Hyper 拥有有界 HTTP/1 连接生命周期；Axum 提供 body/response 类型            |
| 上游 HTTP  | reqwest                     | 复用连接池与流式 body；选用 Rustls TLS 路径                                 |
| TLS        | Rustls                      | 优先由 HTTP 客户端集成，不为列依赖而重复直接依赖                            |
| JSON       | Serde、serde_json           | 静态配置采用强类型；业务 JSON 保留动态结构                                  |
| 日志       | tracing、tracing-subscriber | 请求上下文和结构化错误                                                      |
| YAML       | serde-saphyr                | 当前优先候选；只启用所需解析能力，实施时核对新版本与 MSRV                   |
| JavaScript | rquickjs                    | 使用异步/Promise 桥接能力；已有本机调度、取消与生命周期及 Linux CI 构建证据 |

Tokio、Axum、reqwest、Rustls、serde_json 与 tracing
的官方发布/文档已核验；具体依赖版本在实现时从注册表和上游发布记录再次核对，由
`Cargo.lock` 记录实际解析结果。

YAML 选型不能只把 `serde_yaml` 换成名字相近的 fork：

- `serde_yaml` 已标记 deprecated。
- `serde_yml`
  当前也标记停止维护，其最新兼容版本转发到另一实现，不作为新项目的默认选择。
- `serde_yaml_ng`
  保留原库风格，但原始设计调查时看到的最新发布仍较旧，不能只凭名称断言维护活跃。
- `serde-saphyr`
  提供强类型反序列化、错误位置和解析预算，较符合小型服务配置；项目演进较快，仍需对实际采用版本验证。

rquickjs 官方将绑定描述为 mostly stable，同时说明实验性 feature
的限制。首版不依赖实验性的并行执行能力，也不把该状态当作本项目已经通过测试的证明。

reqwest 的 TLS feature 名称和默认行为会随主次版本变化；以实际采用版本的官方
feature 文档为准，不照抄旧示例的 feature 字符串。

## 12. Workspace 与模块边界

### 12.1 当前结构

主应用 `limen` 位于 workspace 根包，`src/` 包含 CLI、启动组合、HTTP server、gateway、日志和隔离实验入口。
`modules/config/` 对应 Cargo 包 `limen-config`，`modules/script/` 对应 `limen-script`。
目录使用简短职责名，包名保留项目前缀；目录名与 Cargo 包身份不要求相同。
全部依赖（包括内部路径依赖）由根清单集中声明，成员通过 `workspace = true` 使用。

`modules/chat/` 对应 `limen-chat`，隔离纯协议状态机与跨形态转换。
公开错误码与状态映射属于根包，配置库只保留配置职责。

### 12.2 包边界

保留一个根应用包、三个职责库，以及一个独立的 xtask 开发工具包。

| 位置              | Cargo 包       | 职责                                                                      | 边界                                                              |
| ----------------- | -------------- | ------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| 根包 `src/`       | `limen`        | CLI、启动组合、认证、单次路由、hook 顺序、上下游 HTTP、请求与进程生命周期 | 拥有产品策略与资源协调；初始化日志、处理信号并决定退出            |
| `modules/config/` | `limen-config` | YAML/业务 schema、默认值、provider/model 引用、资源配置和安全配置诊断     | 只加载和校验配置；不求值 JS，不发起 HTTP，不执行路由或管线        |
| `modules/script/` | `limen-script` | 本地 ESM、函数登记、JS context/draft、JSON 桥接、VM 线程、Promise 与预算  | 只提供启动、调用、状态和关闭能力；不选择 provider，不编排请求阶段 |
| `modules/chat/`   | `limen-chat`   | SSE 增量编解码、Chat chunk 语义、JSON↔SSE 转换、有界聚合与结束条件        | 纯逻辑库；不依赖配置、VM、HTTP 客户端或服务框架，不持有网络连接   |
| `xtask/`          | `limen-xtask`  | 隔离 VM/HTTP 实验、实验构建、硬超时、进程树监督与临时文件清理             | 显式开发入口；VM 调用 script 公开 API，HTTP 启动 limen 可执行文件 |

根包是应用组合与策略层，不只保留 `main.rs`。认证、模型表、路由和 provider → model 阶段次序只有当前应用这一调用方，
拆成独立 crate 会增加接口与跨包协调，却没有独立的依赖或复用收益，先保留为根包内部模块。
`config` 的拆分隔离启动 schema，`script` 的拆分隔离 VM 所有权与引擎依赖，`chat` 的拆分隔离协议状态机与转换逻辑；
这些边界能分别用纯逻辑测试或隔离运行实验验证。
xtask 不反向成为应用或职责库的依赖。根包保留为默认 workspace 成员，普通应用构建仅包含产品二进制；
开发实验通过 `cargo xtask probe runtime` 或 `cargo xtask probe http` 显式运行，保持与普通单元测试分离。

不为认证、provider、model、limits、错误、日志或通用 SSE framing 单独建包，也不建立兜底的 `common`/`core` 包。
SSE 解码复用 `sse-parse`，逐字节推进避免单网络块形成无界事件队列，overflow 转为明确失败；
`chat` 只增加本项目需要的预算、Chat 语义与转换边界。
只有出现真实的独立调用方或依赖隔离需求，才重新评估更多 crate。

### 12.3 根包内部职责

保留顶层 `cli`、`bootstrap`、`server`、`gateway`、`telemetry` 模块，网关按阶段与生命周期拆分。

- `cli`：参数、帮助与参数退出语义。`bootstrap`：配置装载、脚本启动、配置校验、资源选项转换及失败清理。
- `server`：监听和 HTTP/1 连接驱动、头部与连接预算、断开通知、信号及 VM 退役后的有界关闭。
- `gateway`：请求许可、ID、任务追踪、请求 deadline 与终态；`gateway/lifecycle` 保存单请求状态和统计。
- `gateway/logic`：认证、请求校验、显式模型表、浅层 defaults 和单次路由；规则增长后再按认证、路由、头部拆分。
- `gateway/json`：最终 JSON 有界编码与无需输出缓冲的字节计数；编码结果复用于发送和统计。
- `gateway/pipeline`：全局与 provider → model 的阶段调度、成功 draft/state 提交、四种传输路径选择。
- `gateway/upstream`：原始 status/media、响应阶段、四路径收取与转换；客户端、连接池与单次发送由 gateway/pipeline 组合。
- `gateway/response`：JSON/SSE 下游表示、别名默认、完整响应 hook、有界流 body 与流开始前后的错误输出。
- `gateway/events`：流 hook 顺序、成功输出提交和贯穿两阶段的单源事件展开预算。
- `gateway/observe`：`onError`/`onFinish` 与有限清理预算；流路径按实际完成、失败或截断进入终态。
- `gateway/error`：稳定的应用错误码、失败分类、保留来源及安全对外错误对象。
- `telemetry`：subscriber、panic hook 和应用边界失败报告。

`ErrorCode` 和 `validate_response_status` 位于 `gateway/error`。配置错误留在 `config`，脚本错误留在 `script`，
协议/转换错误留在 `chat`，由根包映射为应用失败并保留原始来源；不另设共享错误包。

### 12.4 库内模块与依赖方向

`config` 保留 `server`、`business`、`error`；静态配置只保存 owned 数据与 hook ID。
`script` 保留 `loader`、`host`、`json`、`context`、`engine`、`error`，
`host` 集中安装公开宿主 API，`host/env` 实现环境变量读取；`engine/startup` 与 `engine/scheduler` 分工不变。
`lib.rs` 的公开调用与通道入口只有在职责或维护需要时再移入 `runtime.rs`，不把 VM 内部类型公开给根包。

`chat` 库内部按 `sse`（分块解码与 framing）、`chunk`（Chat 事件与结束校验）、
`aggregate`（按 choice/tool index 聚合）、`convert`（完整 JSON 转事件）、`error` 组织。
同形态保留合法扩展字段；跨形态支持范围仍由第 7 节约束。转换输入、输出和状态均受显式预算约束，
JSON 转事件采用有界迭代或分批输出，不返回无界事件集合。
聚合器以已有 choice 的 finish reason 作为完成状态的唯一来源；根包对原始流与 hook 变换后流的验证仍各自保留，
不跨越 hook 修改边界共用完成状态。

依赖方向为根包 → `config`、`script`、`chat`，三个职责库之间保持独立，也不反向依赖根包。
根包将 `ResourceLimits` 转为各库实际需要的选项；各库执行自身队列、VM、解析或聚合边界，
根包执行跨步骤事件展开总预算、请求并发、整体 deadline 与上下游背压。
hook 的 `replaceEvents` 方法和单次调用约束属于 `script` 的 draft 契约，展开事件如何进入后续步骤属于根包管线。
`chat` 只消费 owned 字节/JSON/事件与预算，报告结束或截断；根包决定何时读取、调用 hook、写出、取消和观察。

整个拆分沿用 `foo.rs` 加 `foo/` 子模块，单元测试随被测职责迁移至对应 `tests.rs`。
目录与操作命令见[开发指引](development.md)，剩余工作只在[路线图](roadmap.md)中跟踪。

## 13. 实施边界与验证清单

建议分四个可验收部分推进：

1. 配置装载与 `check`：基础配置、ESM、导出验证、hook/Promise
   引用生命周期及异步调度最小验证。
2. 最小网关：显式模型表、独立认证、单次路由、JSON 请求/响应处理。
3. 有序变换：provider/model 顺序 await、header draft、异步提交边界、资源预算。
4. 流处理：SSE 解析、四种传输组合、usage/tool call 边界、取消与截断错误。

HTTP 四路径与请求/响应变换已有 Windows 和 Linux CI 合成上游运行证据，目标部署环境的验收仍单独安排。
纯逻辑测试、隔离 JS 运行实验与网络实验分开记录，
继续用可独立确定的语义检验契约，例如：

- 同一字段先 provider 后 model 修改，模型后的值最终发送。
- 客户端 bearer 不泄漏到上游；认证与普通头冲突能定位。
- 未显式配置 URL hook 时，完整 endpoint 不被拼接或重写路径。
- `null` 不触发默认值；未知模型拒绝；模型路由仅执行一次。
- 同步 hook 与 `Promise<void>` 都按同一提交规则工作；Promise rejection、非 void
  完成值、循环结果和不安全整数得到明确错误。
- 某请求等待 Promise
  时，其他请求、完成通知及取消事件仍可推进；同请求步骤与流事件不乱序。
- 普通等待超时只放弃该调用，下一请求仍可完成；迟到 draft
  不提交。共享闭包可能继续变化的限制被准确保留，不假称 Promise 已取消。
- 永不完成 Promise、同步死循环、无限微任务、取消后晚到结果及观察 hook
  超时都有可验收的结束行为。
- SSE 跨分块、usage 尾块、tool call 多分片、缺失结束标记处理正确。
- JSON ↔ SSE 转换遇到未支持结构时不静默丢失数据。

构建、格式与 lint
不等于行为测试。真实上游接入、性能与端到端体验按部署阶段单独验收，不能在仅有文档和
API 核验时宣布通过。

## 14. 参考来源

- [WHATWG SSE 解析规范](https://html.spec.whatwg.org/multipage/server-sent-events.html#parsing-an-event-stream)
- [OpenAI Chat Completions 官方参考](https://developers.openai.com/api/reference/resources/chat)
- [Tokio 发布与维护说明](https://docs.rs/crate/tokio/latest)
- [Axum 官方文档](https://docs.rs/axum/latest/axum/)
- [reqwest 官方发布记录](https://github.com/seanmonstar/reqwest/releases)
- [reqwest feature 文档](https://docs.rs/crate/reqwest/latest/features)
- [Rustls 官方发布记录](https://github.com/rustls/rustls/releases)
- [serde_json 官方发布记录](https://github.com/serde-rs/json/releases)
- [tracing 官方文档](https://docs.rs/tracing/latest/tracing/)
- [serde_yaml 弃用状态](https://docs.rs/crate/serde_yaml/latest)
- [serde_yml 停止维护与迁移说明](https://docs.rs/crate/serde_yml/latest)
- [serde_yaml_ng 项目与发布信息](https://github.com/acatton/serde-yaml-ng)
- [serde-saphyr 官方文档](https://docs.rs/crate/serde-saphyr/latest)
- [rquickjs 功能、状态与平台说明](https://docs.rs/crate/rquickjs/latest)
- [rquickjs Runtime 中断、内存与加载 API](https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html)
- [rquickjs AsyncRuntime API](https://docs.rs/rquickjs/latest/rquickjs/struct.AsyncRuntime.html)
- [rquickjs 异步 runtime 源码](https://github.com/DelSkayn/rquickjs/blob/master/core/src/runtime/async.rs)
- [rquickjs async_with 调度源码](https://github.com/DelSkayn/rquickjs/blob/master/core/src/context/async/future.rs)
- [rquickjs AsyncContext API](https://docs.rs/rquickjs/latest/rquickjs/struct.AsyncContext.html)
- [rquickjs Promise API](https://docs.rs/rquickjs/latest/rquickjs/struct.Promise.html)
- [rquickjs Module API](https://docs.rs/rquickjs/latest/rquickjs/struct.Module.html)
- [rquickjs Persistent API](https://docs.rs/rquickjs/latest/rquickjs/struct.Persistent.html)
- [rquickjs FileResolver 源码](https://github.com/DelSkayn/rquickjs/blob/master/core/src/loader/file_resolver.rs)
- [rquickjs ScriptLoader 源码](https://github.com/DelSkayn/rquickjs/blob/master/core/src/loader/script_loader.rs)
- [rquickjs-serde 数字转换源码](https://github.com/rquickjs/rquickjs-serde/blob/main/src/ser.rs)
