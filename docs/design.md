# 架构与约束

本文记录当前实现及持续有效的架构约束。配置、API 和 hook 用法见[文档导航](README.md)，未完成事项只放在[路线图](roadmap.md)。[持久化与管理能力重构](#持久化与管理能力重构)记录此次重构采用的契约；运行与部署验收按实际环境分别确认。

## 职责与依赖方向

| 部分              | 职责                                                         |
| ----------------- | ------------------------------------------------------------ |
| 根包 `src/`       | CLI、启动、HTTP 服务、请求生命周期、上游调用、离线管线与诊断 |
| `modules/config`  | 服务配置解析、业务配置验证、共享 JSON 和 URL 边界            |
| `modules/script`  | 本地 ESM 加载、QuickJS 所有权、hook 调度及 JS/JSON 桥接      |
| `modules/chat`    | 有界 SSE 分帧、Chat 事件解释、完成状态和字段级 JSON/SSE 转换 |
| `modules/storage` | SQLite schema、记录、保留清理和有界只读查询                  |
| `web/`            | 同源只读管理界面、浏览器凭据存储和 hash 导航                 |
| `xtask`           | 受父进程监督的隔离测试与进程树清理                           |

主应用组合四个职责库；库不初始化进程日志或决定进程退出。依赖和 lint 在 workspace 根集中管理，工程规则见 [`AGENTS.md`](../AGENTS.md)。

## 请求路径

```text
认证与输入校验 → onRequest → 路由一次 → defaults 合并
  → provider / model beforeSend → 单次上游请求
  → 固定上游状态与媒体类型 → provider / model afterReceive
  → 有界 JSON / SSE 分帧 → Chat 协议解释 → 业务正文 hook
  → 同形保留或字段级 JSON/SSE 转换 → 下游响应
  → 有界终结处理：未发响应的失败经 onErrorResponse → 错误响应
  → 失败时 onError，随后 onFinish
```

路由后的每个管线阶段都按 provider → model 正序执行，同一管线按声明顺序执行。`beforeSend` 可修改最终 URL、模型与上游流参数，但不重新选取管线。客户端最初的 `stream` 意图和上游原始状态、媒体类型作为事实保留，不能被后续草稿覆盖。

真实 HTTP 与离线测试复用网关管线，仅替换上游传输。协议判断和策略提交不能在测试器中维护另一套实现。

## 正文协议与适配边界

SSE 分帧只负责 UTF-8、行和空行边界以及事件字段，不解释 Chat、不把 EOF 当协议完成。原始 frame 与数据视图并存；同形输出保留注释、命名事件、ID、retry 和未知 JSON 扩展，别名或 hook 改写只重编码相应数据。原始载荷保留不承诺改写后的字节完全相同。hook 展开只在流开始保留一次源 BOM，实际编码后的外层字段也计入每个源事件的输出预算。

Chat 解释层独立于分帧和业务 hook，区分生成增量、累计用量、标注、附加元数据、错误和终止。元数据可缺省；非生成事件不必伪造 delta 或 finish_reason。已完成 choice 可以接收不增加生成内容的尾事件，包括音频到期时间、标注和统计，但不能继续追加正文、工具参数或推理内容。未知扩展可同形保留，不能据未知数据推断生成已完成。

客户端 Chat 请求与上游编码是两个边界：现阶段只编码 Chat Completions；未来 Messages/Gemini 的请求编码与响应解释应放在协议适配层，不通过修改原始媒体事实或让业务 hook 模拟原生传输实现。这两个原生协议不属于当前支持范围，适配边界也不要求覆盖全部供应商的通用中间模型。

JSON → JSON 保持开放对象契约。SSE → SSE 不以跨形转换的字段白名单限制可传递数据。跨形转换依字段语义处理文本、内容块、推理、工具、logprobs、音频与累计 usage；传输专用字段有显式处置规则。没有已知合并语义的扩展在同形保留，跨形明确拒绝，不能通用深合并或静默丢弃。JSON 合成 SSE 只保证数据表达，不恢复真实 token 时序。

HTTP 读取结束、协议终止、choice 完成和用量质量分别记录。Chat 流成功仍要求有效 `[DONE]` 与全部已观察生成 choice 完成；finish_reason 不覆盖网络错误或残帧。SSE 注释可作为下游存活信号，发送后 HTTP 状态已经提交；心跳不计入生成内容，也不延长总期限。原生累计 usage 取最新有效快照而不求和，正常增长不判冲突；计数回退与非法计数影响观测质量。

协议解释不以供应商名分支，差异按事件与字段的语义表达。范围内的行为测试覆盖分帧、字段合并、hook 提交、离线组合与受监督的合成 HTTP；真实供应商与 Docker 运行验收各自独立。

## JavaScript 所有权与提交

每个服务生命周期只加载一次业务模块，保留其闭包和 hook。QuickJS 的 VM、上下文、Promise、函数与持久句柄全部由同一个专用线程创建、使用和销毁；跨线程只传 owned JSON、ID、取消信号和终态。

每次 hook 收到私有草稿。只有函数或 Promise 成功结束、返回值满足契约、草稿通过校验且调用仍有效，Rust 才提交结果。失败不回滚已完成的前序 hook，也不能回滚 JS 闭包或全局变量的副作用。请求状态必须放在 `ctx.state`，不能依靠共享闭包隔离请求。

取消与超时会撤销结果的提交资格；停止等待 Promise 不代表它的 JS 工作已经停止。同步执行预算、连续微任务预算、内存限制和调度容量分别约束 VM 工作。运行时退役是服务故障，应用停止接入并退出，由进程管理器决定重启。

## 资源与终态

HTTP 连接、在途请求、VM 命令队列、pending Promise、模块源码、JSON 树、事件展开、输出和聚合均有界。流使用有界通道施加背压；在途许可覆盖终结观察，取消后的清理不能产生无界后台任务。

SSE 成功必须同时满足协议结束和所有生成 choice 完成；尾部元数据不会重新打开生成状态。截断、转换无法保真、脚本产出不合法或资源超限均是失败，不能补 `[DONE]` 伪装成功。响应已开始时只能发送流内错误或关闭连接，不能改写已发送的 HTTP 状态。分帧、原始载荷、hook 扩展、累计聚合和输出均保留各自边界，不为保真缓存完整流。

上游 HTTP 3xx/4xx/5xx 默认保留状态、媒体类型与有界原始正文，不按 Chat 成功协议解析或转换；网关自身错误使用固定 envelope。重定向不跟随。错误响应可以通过 `onErrorResponse` 按 provider → model → 全局顺序适配，但不能将失败变成成功；响应已经开始时跳过该阶段。

`onErrorResponse`、错误写出、`onError` 和 `onFinish` 共享有限清理预算。错误响应策略逐次原子提交正文、状态、头、state 和观测；失败后停止当前适配链，保留主错误及最后有效响应，不递归处理。`onError`、`onFinish` 只观察已确定的终态，不能覆盖结果；VM 已不可用时不能承诺执行这些 hook。

原生用量在脚本改写前提取，来源、计数质量和上游完成事实分别保留。请求统计在终态冻结，日志、观察上下文和离线断言使用同一份数据。耗时区分队列、初次调用、Promise 等待和桥接；共享微任务不能被当成某个请求的 CPU 时间。取消保留已等待的 wall 时间，未取得的细分用样本数表达，清理阶段不回写冻结的请求统计。

受控观测只接受服务配置注册的事件名。每次 hook 先积累私有候选批次，经网关阶段校验后与状态一起提交；有效策略拒绝也提交，失败或失效调用不提交。事件只带 ID 跨线程，终结时由 Rust 补充关联元数据、聚合次数并输出。每调用与每请求均有容量限制，溢出只丢弃观测；观察 hook 的批次仍受共享清理预算约束。

策略复用通过本地纯 ESM 工厂返回现有 pipeline step，沿用相同的执行顺序、校验与提交边界。工厂捕获配置副本，请求数据只留在当前上下文；直接导入和打包使用同一份库源码。

## 信任与范围

业务脚本是本机可信配置，能读取进程环境；VM 的能力限制与资源预算不构成运行任意不可信代码的安全边界。客户端认证和上游认证独立，客户端头只按显式白名单暴露给脚本，转发必须由策略明确构造。

诊断只输出批准的元数据和错误分类，不记录正文、令牌、Cookie、原始 query、任意异常消息或携密源码。目标地址只保留 origin。上游 HTTP 错误正文属于可透传的响应数据；原始网络、解析和脚本异常仅供内部类型化处理，不能直接格式化到日志或客户端。

推理接口范围为 Chat Completions 和显式模型列表；不增加 Responses、自动重试、故障转移或热重载。持久化和管理 API 按下述重构设计推进，浏览器界面只消费只读管理能力。单次生成请求不跟随重定向，也不自动重试。

## 浏览器管理界面

`serve` 在同一监听地址提供构建时内嵌的前端。`/` 返回首页，`/assets/` 只返回已内嵌且媒体类型受支持的构建文件，`/favicon.svg` 返回图标；页面导航使用 hash，不增加服务端页面路由。`/v1` 与 `/admin/v1` 命名空间优先走原有认证和分派；未知路径、缺失资源不会回退到首页。静态请求不进入生成并发、业务脚本或用量记录。

公开首页为静态 LLM 科普文章，页脚的 `※` 按钮打开管理登录。入口的隐藏方式只影响呈现；每个管理请求仍携带独立的 managerKey，由后端认证。管理关闭时首页仍可读取。前端不增加 Cookie 会话、账户或生成权限。

managerKey 由统一浏览器存储模块保存到当前 origin 的 localStorage；加载保存的 key 后先通过 status 验证，才展示管理数据。退出或收到 401 时清除保存的凭据、取消请求并清空查询缓存；已取消请求的晚到结果不能恢复旧会话。存储失败可见，不能声称已经保存或删除。凭据不写入 URL、查询缓存键、诊断或页面正文。

管理界面提供用量、请求记录、运行日志、系统状态和配置五个顶栏入口，采用黑白灰样式；解释放在可聚焦、可点击的帮助标记中。列表与查询都遵守现有分页及时间边界，不自动遍历历史、不自动重试或订阅日志。数值字符串按整数精度保留，缺失与零分开；图表不能凭缺失值补造完整性。

[`api/admin.openapi.json`](../api/admin.openapi.json)是管理交互的机器可读契约，前端客户端由其生成。后端行为变化时同步更新契约；生成一致性检查与真实响应行为验证各自承担不同责任。构建、工具与检查入口统一见[开发指引](development.md)。

主要实现入口：[`src/gateway/`](../src/gateway/)、[`modules/script/src/engine.rs`](../modules/script/src/engine.rs)、[`modules/chat/src/`](../modules/chat/src/)。

## 持久化与管理能力重构

### 目标与适用状态

本节规定生成管线的持久化、管理、日志和启停边界。代码已按此契约实现，剩余环境验收只放在[路线图](roadmap.md)。

用户已确认的边界：

- usage 持久化到数据库，运行日志使用滚动文件，配置、数据和日志分目录管理。
- `apiKey`、`managerKey` 与上游凭据独立；首版管理能力只读检视配置、usage 和日志，配置写入另行设计。
- **本次多阶段重构期间不部署软件，各阶段无需兼容旧版本或前一阶段的配置、目录、内部 API、数据库与日志格式。** 直接替换旧设计，不保留旧字段别名、旧目录探测、双读双写、历史迁移链或兼容兜底；示例和测试随目标契约更新。
- usage 优先可追踪：存储不能接收记录时拒绝新生成请求；已开始请求可以留下结果未知的记录。不为未知引入补偿、重放、备用存储或无限重试。

以下是采用的设计决定。无兼容要求不取消 Chat 协议、敏感信息、JS 所有权、有界资源及流终态约束。WebUI、配置写入、账户/RBAC、计费、在线重启和热重载均不在本节范围。

### 组件与依赖

采用本机 SQLite 与 `rusqlite`，不引入 SQLx、外部数据库服务或通用数据库抽象。`modules/storage` 拥有 schema、数据库连接、事务和查询；不依赖 HTTP、QuickJS，不初始化进程日志或决定进程退出。根应用负责认证、HTTP 分派、将请求生命周期转成 owned 记录，以及存储健康和服务生命周期；滚动日志仍由根应用 telemetry 管理。

存储使用一个写线程和一个只读查询线程，各自持有连接与有界命令通道。查询线程只在管理功能启用时启动，避免统计查询占用写入通道，不建立连接池。连接和 statement 留在所属线程，跨线程只传受控记录、查询条件及 owned 结果。事务中不等待 HTTP、JS 或下游。

| 用途           | 选定能力           | 接入要求                                                                                                |
| -------------- | ------------------ | ------------------------------------------------------------------------------------------------------- |
| SQLite         | `rusqlite`         | 关闭默认 features，启用 `bundled`、`hooks`、`limits`、`fallible_uint`；不用动态扩展加载、ORM 或迁移框架 |
| 日志线程与队列 | `tracing-appender` | lossy 有界队列，应用持有 guard；只封装记录边界、错误状态与格式                                          |
| 大小滚动       | `rolling-file`     | 固定大小条件和备份数量，不自建轮换器                                                                    |
| 时间           | `chrono`           | 显式依赖并集中管理，复用滚动库已有的时间依赖；输出 UTC，不引入时区数据库                                |
| 运行标识       | `uuid` 的 `v4`     | 每次运行创建随机 run ID，与单调递增序号组成请求和日志 ID                                                |
| 密钥比较       | `subtle`           | 使用现成常量时间比较接口，不自行编写密码学代码                                                          |

上述库接入时已复核注册表、上游发布、features、MSRV 与锁文件；`fallible_uint` 用于受范围检查的 u64 SQL 转换。Rust 1.99、Windows MSVC 和 Linux C 工具链仍是构建基线，不把本机验证代替容器验收。资料见[依据与验证边界](#依据与验证边界)。

### 服务配置与命令

服务 TOML/YAML 继续负责运行参数，`config.js` 及本地 ESM 继续负责 provider、模型与策略。配置不存入数据库，启动时加载一次，修改后重启。

当前服务配置示例如下：

```toml
listen = "127.0.0.1:8080"

[auth]
api_key = "replace-with-a-random-api-key-at-least-32-bytes"
manager_key = "replace-with-a-different-random-manager-key"

[storage]
dir = "../data"
retention_days = 90
max_records = 100000
max_db_bytes = 1073741824
operation_ms = 1000

[log]
level = "info"
format = "text" # stderr 格式；文件始终为 JSONL
dir = "../logs"
max_file_bytes = 16777216
max_files = 8 # 备份数量，另有一个活动文件

[management]
concurrency = 4
timeout_ms = 2000
```

`auth.api_key` 必填；`auth.manager_key` 缺省表示管理功能关闭，空字符串不表示关闭。每个 key 为 32–256 字节、无空白的可打印 ASCII，二者不能相同。不支持 `client_auth`、匿名模式或旧 `log.output`；未知字段、重复键和非法值直接报配置错误。服务配置不做环境变量插值；上游仍可由 JS 的 `limen.env` 读取独立凭据。

省略 storage/log/management 的可选字段时采用上例默认值。保留现有 `[limits]`、`[script_context]` 和 `[observability]`；密钥不属于可被脚本读取的客户端头或上下文。

| 字段                     | 可接受范围                               |
| ------------------------ | ---------------------------------------- |
| `storage.retention_days` | 1–3650 天                                |
| `storage.max_records`    | 1000–10000000，且大于生成并发数          |
| `storage.max_db_bytes`   | 64 MiB–64 GiB，4096 字节的整数倍         |
| `storage.operation_ms`   | 50–10000 ms，且小于 `limits.shutdown_ms` |
| `log.max_file_bytes`     | 1–256 MiB                                |
| `log.max_files`          | 1–32 个备份文件                          |
| `management.concurrency` | 1–16                                     |
| `management.timeout_ms`  | 100–10000 ms                             |

目录路径不得为空；相对路径以选定服务配置文件的父目录为基准。config、data、logs 解析到的目录必须分离，不能相同或互相包含，防止清理与权限作用域混淆。`serve` 创建缺失的 data/logs 目录，但不创建示例配置、不改 owner/group；现有路径和权限不满足要求则报错。

未传 `--config` 时只读取 `./config/server.toml`，不存在直接报错；YAML/YML 通过 `--config` 显式选择，不做扩展名或旧目录回退。`config.js` 相对服务文件的位置不变。本机从仓库运行时显式指定 `--config local/config/server.toml`，容器以 `/` 为工作目录使用相同规则。

- `serve`：初始化持久化、滚动日志、JS 和 HTTP。文件日志就绪前的启动错误只写受控 stderr。
- `check`：解析配置、执行顶层 JS 并验证业务配置；不打开/创建运行数据库，不创建日志，不执行清理。通过不表示运行目录可写。
- `test`：继续使用合成上游和相同网关管线，注入内存记录接收端；不访问真实运行数据库或日志目录。持久化失败也通过同一接收边界注入。

### 目录、权限与部署边界

```text
部署目录/                 local/
  config/                   config/
    server.toml               server.toml
    config.js                 config.js
    ...本地导入模块           ...本地导入模块
  data/                     data/
    limen.sqlite3             limen.sqlite3
    limen.lock                limen.lock
    ...SQLite 附属文件        ...SQLite 附属文件
  logs/                     logs/
    limen.jsonl               limen.jsonl
    limen.jsonl.1             limen.jsonl.1
    ...滚动备份               ...滚动备份
```

| 宿主目录   | 容器挂载     | 权限                                             |
| ---------- | ------------ | ------------------------------------------------ |
| `./config` | `/config:ro` | UID/GID 10001 可读、目录可遍历                   |
| `./data`   | `/data:rw`   | UID/GID 10001 可读写，包括数据库附属文件和实例锁 |
| `./logs`   | `/logs:rw`   | UID/GID 10001 可读写                             |

容器配置使用绝对 `/data` 和 `/logs`，监听 `0.0.0.0:8080`；宿主端口仍默认绑定 `127.0.0.1:8080`。容器直接以 UID/GID 10001 运行，根文件系统只读，保留 `cap_drop: ALL` 与 `no-new-privileges`，删除当前 root 改权入口、相应 `cap_add` 和仅为 `setpriv` 安装的运行时依赖。部署者预先准备挂载权限；不自动提权、递归改权、搬运或复制旧配置。

仓库忽略根目录的 `config/`、`data/`、`logs/` 和 `local/`；不按任意目录名全局忽略，避免掩盖 `modules/config` 等源码。部署示例保持脱敏，提交前仍检查暂存区。

重构期间，CI 的部署配置检查与 OliveTin webhook 调用均以注释停用，触发脚本保留供明确要求恢复部署时使用；构建、测试和镜像发布继续运行。不要靠删一部分 Secrets 达到停部署。仓库之外跟踪 `latest` 的自动更新需在推送实施提交前停用或固定镜像 digest；这是部署环境操作，不假定仓库修改能替代它。完成重构也不自动恢复部署，只有用户明确要求时重新启用。

备份采用停机后复制完整 data 目录，并独立备份 config；恢复时保持服务停止、恢复目录权限、启动校验 schema。首版不提供在线备份 API，也不把日志当恢复数据库的来源。运行中仅复制主数据库不能保证一致性。[SQLite 备份说明](https://sqlite.org/backup.html)

### 数据库契约

使用 WAL、`synchronous=FULL`、4096 字节页和单实例数据目录；不支持 NFS/SMB 等网络共享存储。启动持有 `limen.lock` 的操作系统排他文件锁至连接全部结束，采用标准库 `File::try_lock`；文件存在本身不代表锁被占用，不按 PID 猜测或删除锁来接管实例。[SQLite WAL](https://www.sqlite.org/wal.html)、[Rust 文件锁](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)

库不存在时在事务内创建当前 schema；已有库通过固定 application ID 和 `user_version` 校验，错误、损坏或版本不匹配均启动失败。不静默删库，不把“无法识别”当作空库。重构阶段改变不兼容 schema 就递增内部版本，由开发者显式重建数据。首个恢复部署版本所使用的 schema 成为受支持基线；之后遇到真实升级需求再提供对应向前迁移，不回补重构历史、不预建迁移框架。

主表为 `requests`，没有日志表、密钥表、脚本表或预聚合表。请求 ID 为 `limen-r-{run_id}-{sequence}`，管理请求与日志分别使用 `limen-m-`、`limen-l-` 前缀和独立序号；run ID 是 UUID v4，不使用凭据或客户端输入生成 ID。数据库另有自增且不复用的 `record_id`，供分页与保留排序使用。

| 列组       | 确定内容                                                                                                                             |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| 身份       | `record_id`、唯一 `request_id`、`run_id`                                                                                             |
| 时间       | `started_at_ms`、可空 `finished_at_ms`；UTC Unix 毫秒，耗时来自单调时钟，不用墙钟相减                                                |
| 记录状态   | `state = pending / finalized / interrupted`                                                                                          |
| 管线结果   | 可空 `outcome = success / failed / cancelled`，可空受控 `error_code / error_phase / error_step`                                      |
| 路由       | 可空 `model / provider / configured_upstream_model / final_upstream_model`                                                           |
| 上游事实   | `client_stream`，可空 `upstream_status / upstream_media / upstream_completed`；媒体类型只保存规范化类型，不存原始头                  |
| 原生 usage | 五个可空 token 计数，以及可空 `usage_source / usage_quality`                                                                         |
| 摘要       | `input_bytes / outbound_bytes / upstream_bytes / output_bytes`、`request_ms / upstream_headers_ms / first_output_ms`，未取得的值为空 |

`pending` 初始行仅有身份、开始时间、原始客户端 stream 和已读入站字节。终态提交一次性补齐允许字段并改为 finalized；finished 时间与 outcome 表示网关管线终态，不证明客户端已经收到所有字节。启动取得独占锁后，将遗留 pending 分批改为 interrupted，不伪造结束时间、结果、用量质量或上游完成事实。状态只允许 pending → finalized/interrupted；不使用覆盖式 upsert 隐藏重复终结，冲突返回受控存储错误。

字符串列沿用业务标识约束；可能由 hook 改写的最终模型限定为 1024 字节，不符合时在对应草稿提交处拒绝。受控记录总编码上限 16 KiB，不存正文、完整 URL、任意请求头、原始 query、异常文案、`ctx.state` 或整个统计 JSON。上述列是持久化摘要；更细的已有 hook 耗时仍可由离线断言和安全诊断观察，不为落库复制整棵运行状态。

初始索引是唯一 request ID、`(started_at_ms, record_id)`、`(model, started_at_ms, record_id)`、`(provider, started_at_ms, record_id)`；状态和结果在有界时间查询中筛选，不为每种组合建索引。存储 DTO 使用类型化字段，SQL 参数绑定，API 参数不能成为 SQL 片段。

#### usage 口径

继续使用脚本改写前的 `UsageTracker` 与冻结统计。五项为 input、output、total、cached input、reasoning output，缺失保持 NULL。`missing / partial / reported / invalid / conflicting` 沿用现有规则；非法或冲突快照不暴露有效计数，累计上报只取最后有效快照而不逐 chunk 累加。total 不由 input + output 补算，缺少任一项不使其他已知项消失。

失败或取消请求也纳入已知消耗统计，是否上游完成单独展示。pending/interrupted 没有终态计数，既不是零消耗，也不是已知失败。汇总每个计数字段都返回 `sum` 和 `samples`；无有效样本时 sum 为 null，有样本且确为零才返回零。汇总同时返回记录状态、结果、用量质量和上游完成分布，不能把已知小计称为完整账单。

单条 token 计数沿用现有安全整数校验；聚合在 Rust 侧用 checked `u128` 累加，不使用可能溢出的 SQLite 整数 SUM 或浮点 TOTAL。管理 API 中 token/byte 计数及其总和统一用十进制字符串或 null，记录数、分页条数及受限毫秒值用 JSON number。不引入价格、货币或 token 估算。

#### 保留与容量

默认保留 90 天、最多 100000 条，按开始时间删除最旧的 finalized/interrupted，pending 不参与清理；清理后的历史不另存累计汇总。新记录插入前如达到条数上限，最多删除 500 条最旧终态以腾出空间；无可删行则拒绝本次准入。年龄清理在启动及运行时每 60 秒执行，每批最多 500 行、批间允许请求写入，不开启第二条维护任务链。调低条数上限时在启动预算内分批清理到新上限；超时失败，不先接入再无界追赶。

主文件通过 `max_page_count` 限制在 `max_db_bytes` 以内；已有文件大于新配置上限时启动报错。删除释放页供复用，首版不在线 VACUUM，也不承诺主文件随记录减少而缩小。连接缓存各 8 MiB，临时查询内存受 SQLite 限额和下述查询边界控制。

WAL 自动 checkpoint 阈值为 1000 页，复用后保留目标为 16 MiB。`journal_size_limit` 不是 WAL 的硬上限：提交后若 WAL 超过 64 MiB，标记存储故障并停止新生成准入，不循环尝试 checkpoint 恢复接入。已开始请求只做一次终态写入；有限在途事务可能使 WAL 继续增加。主文件额度不冒充整个目录的精确磁盘配额。[SQLite 容量与 journal 设置](https://www.sqlite.org/pragma.html)

### 请求准入、终态与关闭

```text
HTTP 分派 → apiKey 认证 → 生成并发许可 → 有界请求校验
  → pending 提交确认 → onRequest → 一次路由 → 原有生成管线
  → 冻结统计与管线结果
      ├─ 终态持久化（storage.operation_ms）
      └─ 响应/错误适配及观察 hook（原有 cleanup_ms）
  → 两支均结束或到期 → 释放生成许可
```

这里只接纳通过认证、并发准入、媒体类型和请求结构校验的 Chat 生成请求。其他请求不建立 usage 行；`onRequest` 拒绝、未知模型、上游错误、取消与截断仍有终态记录。初始写入确认前不执行 hook、不发送上游；过期后收到确认也不能恢复请求。`GET /v1/models` 仅需 apiKey 和已加载业务配置，不依赖数据库健康，不执行生成 hook。

初始写入等待使用 `min(剩余请求时间, storage.operation_ms)`；冻结时的请求耗时包含这一等待。提交期间客户端断开或服务取消不会丢弃确认等待；有界确认完成后先复核取消与期限，已取消只记录终态，不进入生成管线，不能把正常取消误判为数据库故障。冻结后在同一受跟踪请求任务内并行等待 owned 终态写入与原有响应清理，不产生脱管后台任务。两支各用自己的预算，持久化不抢占当前默认仅 100 ms 的观察清理时间；它也不依赖 `onFinish` 成功或 VM 存活。清理耗时不回写冻结统计。

| 条件                                   | 行为                                                                                         |
| -------------------------------------- | -------------------------------------------------------------------------------------------- |
| 生成并发已满                           | 503 `busy`，不入库、不执行生成管线                                                           |
| 存储已故障，或 begin 写入失败/确认超时 | 503 `storage_unavailable`，无上游调用；实际存储错误/写入确认超时将写健康锁定为 failed 至重启 |
| 有界通道暂时满                         | 本次 503 `busy`，不排队重试，也不因此永久标记磁盘故障                                        |
| 终态写入失败或确认超时                 | 写健康变为 failed，新生成被拒绝；现有网络结果保持原语义，初始行可能保留 pending              |
| 客户端取消或 HTTP 期限结束             | 停止生成，依然对已有记录做一次有限终态提交，不恢复请求                                       |
| DB 查询超时/条件过宽                   | 只失败该管理查询，不把数据库写健康标记 failed                                                |
| DB 读取遇到损坏或 I/O 错误             | 标记存储故障并停止新生成，状态接口仍可说明故障分类                                           |
| 日志队列满或 writer 故障               | 统计丢弃或 I/O 失败，生成继续；日志与 usage 持久性分开                                       |

终态提交确认与网络发送不是原子事务。客户端收到成功而记录仍不完整是允许的未知窗口；不缓存整条 SSE、不逐 token 落库、不追加第二个响应来补偿。取消等待也不代表 SQLite 停止，超时操作可能随后提交；返回的管理记录以已提交事实为准，存储故障状态仍要显式重启恢复，不自动反复探测。

写队列容量为生成并发数加 1，每个在途生成最多有一个数据库命令，额外槽给串行维护；读队列容量为管理并发数。`busy_timeout` 显式设为 0，由应用一次性处理锁冲突，不继承驱动可能较长的默认等待。读 SQL 使用 progress handler 检查期限与取消，Rust 聚合循环也定期检查；只读连接的中断不能误伤写事务。

先同步读取并校验有大小限制的服务配置，再按其中的 `limits.startup_ms` 预算完成目录锁、日志、schema/恢复清理及 JS，全部就绪后绑定监听。该预算不覆盖最初的本机服务配置文件 I/O，不声称能够中断操作系统阻塞的读取。只在管理启用时启动读取服务。storage 初始化失败直接退出，不能进入无数据库服务模式。

关闭顺序为停止监听和准入、取消并等待连接与在途任务、停止并排空存储线程、关闭 VM、最后释放日志 guard。HTTP 排空、存储关闭、VM 关闭分别最多 `limits.shutdown_ms`，两路日志共预留 3 秒；默认总预算 18 秒，Compose `stop_grace_period` 取 25 秒。配置增大时要求宽限时间大于 `3 × shutdown_ms + 3000 ms` 并留调度余量。底层 I/O 不可中断时不无限 join，超预算非零退出，由进程结束释放资源；不声称已成功刷新，也不在仍有存储操作时提前释放实例锁。

### 滚动日志契约

沿用 `tracing` 事件与字段白名单。文件只输出 JSONL，stderr 用配置的 text/JSON；两路都带 UTC 毫秒时间。日志只承担诊断，usage 查询事实来自数据库，不再输出一套完整可供重建用量的统计存档。

每条文件记录包含 `schemaVersion: 1`、`timestamp`、`level`、`logId`、固定 `event`，以及允许的可选 `requestId / phase / code / model / provider` 等诊断字段。log ID 由 run ID 与日志序号组成；固定事件名代替任意异常 message。现有 step、origin 和批准的观测事件仍按既有安全规则记录。

固定单条上限 16 KiB（含换行），在进入队列前一次性编码完整记录。超大事件输出只含关联 ID、原事件类别及 `truncated: true` 的合法摘要，不切断原始 JSON 字节。每路非阻塞队列最多 1024 条、lossy，满时丢弃并计数；禁止改为同步等待空位阻塞 Tokio。默认文件上限 16 MiB、保留 8 个备份，最多为 9 个文件加每文件一个有界记录的超出量；不压缩、不按天滚动。

文件重命名与保留轮换交给 `rolling-file`。该库的自动条件路径会吞掉轮换错误并自行输出 stderr，因此包装层按大小显式调用其可返回错误的 `rollover()`，将错误交给统一日志健康处理；不重复实现轮换算法。`serve` 启动时将非空活动文件滚动一次，再写新运行日志；异常中断留下的半行留在旧文件，检视时计为不可解析记录，不覆盖修复。日志读取与轮换使用短期文件句柄，不能长期 pin 住旧文件。

应用在 writer 外包装轻量 I/O 错误计数和最后受控错误类别；这与 `tracing-appender` 的队列丢弃计数分开。第一次文件写入/轮换失败后停止本次运行的文件输出，后续事件计为丢弃，stderr 和管理状态继续报告，修复后重启；不在每条日志上重新打开文件或重试轮换。相关诊断限频为每 60 秒至多一次，不能递归进入同一个故障输出。stderr 故障同样计数；状态不声称它必然能送达。

guard 由应用入口持有到 VM 结束后，关闭只作有限刷新，强制退出可能丢失尾部；日志不要求逐条 fsync。`tracing-appender` 当前 guard 有有限等待，但握手本身不能证明最后 flush 和 writer 已结束；包装层以 writer 释放通知，在同一 3 秒总预算剩余时间内等待完成，不作无限 join。最终失败诊断先输出，再关闭日志。[非阻塞队列与关闭源码](https://docs.rs/tracing-appender/latest/src/tracing_appender/non_blocking.rs.html)

Docker 对 stderr 单独配置 `local` logging driver，`max-size: 10m`、`max-file: 3`；应用文件轮换不限制 Docker 自己的日志占用。

### HTTP 分派与认证

同一监听地址、两个命名空间：`/v1/*` 使用 apiKey，`/admin/v1/*` 使用 managerKey；key 不互通，managerKey 没有隐含生成权限。管理请求直接进入 Rust 服务，不经过业务脚本或上游认证。无第二监听端口、用户账户、Cookie 会话或 JWT。

认证只接受一个 Bearer Authorization，拒绝重复头和空 token，不从 query/Cookie 读取。合法命名空间内先认证再判方法与端点，认证失败统一 401；管理关闭时 `/admin/v1/*` 返回 404。已认证的错误方法返回 405 与 Allow，未知路径 404。未知顶层路径直接 404，不借它判断密钥有效性。密钥不进入日志、数据库、配置投影或 JS 上下文。

管理并发默认 4，无等待队列，满时 503 `busy`；默认总请求预算 2 秒。它与生成并发许可分离，复用有界 HTTP/1 连接管理，不承诺同一监听端口在连接洪泛下仍提供独立带宽。许可覆盖底层读取的实际生命周期，HTTP 超时不释放一个仍在执行的阻塞读取槽位，不积累脱管的 blocking 任务。请求头和连接期限沿用传输层上限；管理 query 最多 4096 字节，响应最多 1 MiB，GET 不接收非空正文。

管理响应使用 JSON、camelCase、UTC RFC3339 毫秒时间及 `Cache-Control: no-store`，带 `X-Limen-Request-Id`。错误 envelope 沿用 `{error: {code, message, request_id}}`，message 为固定受控文案。除上述 401/404/405 外：非法参数为 400 `invalid_query`，失效游标 410 `cursor_expired`，范围或分组过宽 422 `query_too_broad`，存储不可用 503 `storage_unavailable`，查询期限结束 504 `query_timeout`；不返回部分汇总伪装成功。

不发送跨域许可头，浏览器管理界面使用同源请求；配置改写、重启、文件下载、任意文件路径和 SQL 查询均没有端点。

### 只读管理 API

下表是首版完整端点集合；成功体分别使用 `{data: ...}` 或列表 `{data: [...], nextCursor: string|null}`。管理接口不是 OpenAI 接口，只有推理端点保留原有协议。

| 方法与路径                           | 首版行为                                                                             |
| ------------------------------------ | ------------------------------------------------------------------------------------ |
| `GET /admin/v1/status`               | 进程、VM、存储与日志健康及丢弃/错误计数；仅读内存状态，不为一次探测访问 DB 或执行 JS |
| `GET /admin/v1/requests`             | 生成记录的有界过滤与游标分页                                                         |
| `GET /admin/v1/requests/{requestId}` | 单条持久化摘要；不存在或已清理返回 404，不猜测两者区别                               |
| `GET /admin/v1/usage`                | 一次有界时间窗口内的准确已知用量汇总                                                 |
| `GET /admin/v1/logs/files`           | 当前可检视的应用日志文件清单，不列出目录内其他文件                                   |
| `GET /admin/v1/logs`                 | 单个已知文件内的倒序日志窗口，不提供跨文件全量搜索                                   |
| `GET /admin/v1/config`               | 当前加载配置的白名单投影，按 section 分页                                            |

#### 状态与配置投影

status 返回 `version / runId / startedAt / acceptingGenerations / vm / storage / logging`。storage 包含 `ready / failed / stopping` 状态和最近受控故障分类；logging 分开给 file/stderr 状态、丢弃数、I/O 错误数，生命周期累计计数使用十进制字符串。故障分类不含底层错误字符串、路径或 SQL。记录恢复和保留策略可说明在 storage 状态中；status 成功读取返回 200，即使 `acceptingGenerations=false`，不把可读取状态与可生成混为一谈。

config 的 `section` 为 `server`（默认）、`models` 或 `providers`。server 返回监听、非秘密运行限额、保留策略、日志参数、`apiKeyConfigured / managerKeyConfigured`，不返回实际 key 或路径。models 每项只含别名、provider、配置上游模型；providers 每项只含标识与 endpoint origin。列表默认 50 条、最多 100，按标识排序，游标绑定 run ID，重启后失效。没有“原始配置”模式，不返回 JS、环境变量、headers、defaults、hook 源码或秘密占位片段。

#### 请求查询

from/to 是 RFC3339 时间，采用 `[from, to)`，要求 from 小于 to，按请求开始时间归属；二者都省略时取最近 24 小时，否则必须同时提供，最大跨度 31 天。额外过滤项只有 `model`、`provider`、`state`、`outcome`，各接受单值精确匹配；不支持正则、通配或自由排序。字段缺省意味着不筛选，未知/重复参数、非法枚举直接 400。

列表默认 50 条、最多 100，按 `(started_at_ms, record_id)` 倒序；先达到 1 MiB 响应上限时缩短本页并保留 nextCursor。首个页面保存 record ID 上界，后页排除新插入记录；游标同时绑定已解析的时间窗口、有效过滤条件和最后排序位置。后页可仅传 cursor 与 limit，若重复给过滤项则必须与游标一致，不能重新计算“最近 24 小时”。每页是独立读取，不跨请求持有事务；期间终态更新、保留清理可能使结果变化，不承诺跨页历史快照。request ID 查询不受默认时间窗口限制。

响应记录包括上述持久化字段，外部字段用 camelCase；数值与 null 规则见 usage 口径。分页游标为有版本、大小不超过 2048 字节的 opaque 编码，包含 run ID，重启返回 410；不含秘密，不作为认证凭证。服务端逐项校验字段与范围，不将解码内容直接用于路径或 SQL。

#### usage 汇总

from/to、model、provider 与请求列表相同；默认最近 24 小时，最大 31 天。`groupBy` 仅支持 `none / day / model / provider`，默认 day，每次只能选一个维度，day 按 UTC 日边界；不按请求成功与否默认过滤。未知路由为 null 分组，不丢弃这类记录。

每次最多扫描 100000 条匹配记录、最多 128 组。读取在单个只读事务内逐行处理，Rust 侧只保留有界分组计数；超过任一上限返回 422，用户可缩小窗口，不自动分片或后台重试。SQL 和聚合循环共享该管理请求的剩余期限。

响应 data 含 `from / to / groupBy / retention / groups`。retention 返回保留策略与当前最早保留开始时间，不承诺其之前的完整性。每组含 `key / records / states / outcomes / qualities / upstreamCompleted / tokens`；后五者按固定枚举统计，未知与缺失独立表达。tokens 每项形如 `input: {sum: "123", samples: 2}`，无样本为 `{sum: null, samples: 0}`。pending/interrupted 计入 records 和 states，终态才计入 outcome/quality，未知上游完成单独计数。

#### 日志检视

files 最多返回活动文件加配置数量的备份，顺序为活动、`.1` 到 `.N`；条目含 `fileId / active / sizeBytes / firstTimestamp`。fileId 采用文件首条完整记录的 log ID，读取时重新确认身份，不把可复用的文件名当作身份。空文件不列出；首条记录损坏的文件只报告受控不可读状态，不扫描任意内容推测身份。默认 logs 选活动文件，没有日志时返回空列表。

logs 参数仅有 `fileId / cursor / limit / minLevel / requestId`，默认 100 条、最多 200；minLevel 使用 trace/debug/info/warn/error，包含该级及以上，缺省不筛选。首屏从选定文件尾部向前读取，后页继续向前；cursor 绑定 run ID、fileId、上一轮已消费的扫描起点和过滤条件。后页可仅传 cursor 与 limit，显式重复的文件/过滤参数必须一致。每次最多扫描 1 MiB，响应另受 1 MiB 上限限制，先达到任一预算即结束页面，nextCursor 必须覆盖尚未返回的匹配记录。达到扫描预算可返回空 data 及非空 nextCursor，表示该窗口未匹配，不能冒充文件已读完。到文件开头 nextCursor 为 null，其他历史文件由 files 显式选择。

文件在轮换中更名可以按 fileId 在受管文件集合内定位；已淘汰、被截断、首记录变化或重启则返回 410，不切换到另一文件、不为游标保留备份。单条最多 16 KiB，半行和不可解析行计入 `skippedRecords`，不作为任意文本透出；查询超时整体失败。响应额外给 `fileId / skippedRecords / scannedBytes`，明确过滤只覆盖本次扫描窗口。

文件读取只允许固定活动/备份命名及真实普通文件，拒绝符号链接/重解析点，不接受客户端路径。每次请求短暂打开并复核首记录；轮换竞争造成读取失败时返回受控错误，由客户端重新读取文件清单，不自动重试。首版没有 follow、SSE 日志订阅或全文索引。

### 依据与验证边界

数据库选择基于单实例本机记录与查询需求；WAL 允许读写并发但仍只有一个写者，FULL 提供提交同步。官方说明见 [SQLite WAL](https://www.sqlite.org/wal.html)。SQLite 配置、读查询中断及 driver 默认锁等待依据 [PRAGMA](https://www.sqlite.org/pragma.html) 和 [rusqlite Connection](https://docs.rs/rusqlite/latest/rusqlite/struct.Connection.html)。

日志选择基于现成的有界写线程和大小轮换能力，队列丢弃不等于文件写入成功，采用轻量 writer 状态包装补足观测。[tracing-appender](https://docs.rs/tracing-appender/latest/tracing_appender/)、[rolling-file](https://docs.rs/rolling-file/latest/rolling_file/)

已查注册表与上游的版本快照：rusqlite 0.40.2、tracing-appender 0.2.5、rolling-file 0.2.0、uuid 1.28.0、chrono 0.4.45、subtle 2.6.1。SQLx 不再作为本次实施候选。这里不固定未来 Cargo 的 patch 版本；实际接入重新核对，不因收敛设计就跳过依赖与工具链验证。[注册表](https://crates.io/)、[rusqlite 发布](https://github.com/rusqlite/rusqlite/releases/tag/v0.40.2)、[tracing 发布](https://github.com/tokio-rs/tracing/releases/tag/tracing-appender-0.2.5)、[rolling-file 上游](https://github.com/Axcient/rolling-file-rs)、[UUID 发布](https://github.com/uuid-rs/uuid/releases/tag/v1.28.0)、[chrono 发布](https://github.com/chronotope/chrono/releases/tag/v0.4.45)、[subtle 上游](https://github.com/dalek-cryptography/subtle)

本节额度是首版默认契约，不是性能测试结果。确定性常规测试覆盖配置、权限矩阵、记录转换、汇总/null 语义、游标以及四种 JSON/SSE 离线组合；数据库逻辑可用内存 SQLite。临时文件持久化、锁、磁盘错误、轮换、进程中断、真实 HTTP 和关闭边界放入受监督隔离测试，容器权限与停止宽限单独验证。不编写断言源码或文件布局的测试，不用文档检查代替这些行为验收。未完成的环境验收只记录在路线图。
