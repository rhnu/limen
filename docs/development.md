# 开发指引

## 目录与依赖方向

```text
.
├── src/                 根包：CLI、bootstrap、JSON/SSE gateway、HTTP server 与日志
├── modules/
│   ├── config/          limen-config：TOML/YAML 与业务配置、资源 schema、配置诊断
│   ├── script/          limen-script：ESM、只读环境变量、严格 JSON 桥接和有界 JS 调度
│   └── chat/            limen-chat：SSE 编解码、chunk 校验、四路径转换与有界聚合
├── examples/            无真实凭据的基础配置与共享模块
├── xtask/               limen-xtask：Rust 编写的隔离 VM/HTTP 实验与进程监督
├── .cargo/config.toml   cargo xtask 别名
├── docs/                功能使用指南、设计、唯一实施路线图、开发指引
├── README.md            英文项目介绍与快速开始；中文版为 README.zh-CN.md
├── LICENSE              GNU AGPLv3 完整许可证
├── Cargo.toml           根包、workspace、集中依赖和共享 lint
├── Cargo.lock           应用依赖解析结果，纳入版本控制
├── rust-toolchain.toml  固定 Rust 1.99.0、rustfmt 与 Clippy
└── rusteward.toml       Rusteward 格式与 lint 配置
```

应用组合配置、脚本和 Chat 协议库；配置库不依赖 VM，脚本库不依赖 HTTP 服务框架或配置 schema（使用 http 校验头部）。
Chat 库仅消费 owned 字节与 JSON，不持有连接，也不依赖配置或 VM。
根包 `limen` 是默认 workspace 成员，根目录的 `cargo run`、`cargo build` 和 `cargo test` 默认选择它；
全 workspace 检查与测试继续显式使用 `--workspace`。`limen-xtask` 是独立开发工具包，
通过 `cargo xtask` 显式调用；应用与职责库不依赖它。
跨线程仅传 owned JSON、ID、deadline 与取消信号，库的公开 API 不暴露 rquickjs 句柄。
请求身份位于 `context.meta.requestId`，`HookCall` 仅另带诊断用的调用 ID 与选择函数的 hook ID。
hook ID 在启动时按顺序分配，VM 用只读顺序表持有函数引用，拒绝零值和越界 ID。
脚本公开宿主能力在 `modules/script/src/host.rs` 的 `install` 中统一注册、冻结并发布到 `limen`；
`host/env.rs` 只构造环境读取函数，单元测试随模块保存在 `host/env/tests.rs`。
根包 gateway 模块负责路由、管线与错误，server 模块拥有连接、关闭和 VM 退役传播。
短目录名表示职责，Cargo 包名使用 `limen-` 前缀；路径依赖由根清单显式指定。
公开错误码、状态映射和安全错误对象属于根包 gateway；职责见
[设计中的工程边界](design.md#12-workspace-与模块边界)。
单元测试放在被测模块的 `tests.rs` 子模块，禁止 `mod.rs`。
workspace 默认拒绝 `unsafe`；仅 xtask 的 Windows 控制台模块局部允许原生调用，
每个 unsafe block 必须写明安全依据，Clippy 的 `undocumented_unsafe_blocks` 设置为 deny。

## 本地检查

QuickJS 包含 C 源码，需要可用的 C 编译工具链。Windows 本机已能构建其 MSVC 目标；
Linux 需准备 C compiler、归档器及 libc 开发环境；Linux CI 编译与容器镜像构建已跑通。

在仓库根目录按顺序执行：

```sh
cargo check --workspace --all-targets --locked
cargo dev format --locked
cargo dev lint --locked --deny-warnings
cargo test --workspace --lib --bins --locked
```

Rusteward 不可用时安装：

```sh
cargo install --git https://github.com/rhnu/rusteward.git --locked rusteward
```

`cargo dev format` 会修改源码；只读检查使用 `cargo dev format --check --diff --locked`。
质量规则由 `rusteward.toml` 管理，不以编译或格式通过代替行为验证。
`--lib --bins` 只运行配置、输入边界、SSE/转换、预算和 CLI 等纯逻辑单元测试，不隐式运行集成或文档测试。

文档修改收尾时仅传本次修改的 Markdown 文件，例如：

```sh
rumdl check --fix README.md README.zh-CN.md docs/features.md docs/design.md docs/roadmap.md docs/development.md
```

本机使用 rumdl 0.2.79，CI 同版本。可用时另运行 `actionlint .github/workflows/ci.yml`，并执行 `git diff --check`。
actionlint 1.7.12 尚不识别 GitHub 已支持的 `concurrency.queue`，该版本可仅忽略这一诊断：

```sh
actionlint -ignore 'unexpected key "queue" for "concurrency" section' .github/workflows/ci.yml
```

## 隔离运行时实验

JS 执行实验与普通单元测试分开。xtask 通过内部 worker 子命令将每个实验放入独立进程，
超过外部时间上限即终止，避免同步死循环或 VM 故障让单元测试及 CI 无界挂起。无需 Python。

```sh
cargo xtask probe runtime
cargo xtask probe runtime context-stream
cargo xtask probe runtime environment
cargo xtask probe runtime --list
```

使用 `cargo xtask probe runtime --help` 查看单项实验、超时和列表选项。
`--timeout` 默认 12 秒，接受 1–60 秒；`--list` 不启动实验 worker。
Cargo 首次仍需构建 xtask。内部 `__worker` 等子命令供监督器调用，不应绕过外部超时保护直接使用。
实验只使用临时合成配置，不连接真实 provider，验证 VM 行为；Docker 部署与 HTTP 端到端验收分别执行。
临时目录由 xtask 父进程统一持有和清理，覆盖正常完成、失败与外部超时终止。
父进程监听 Ctrl-C 和 Unix SIGTERM，停止并回收进程树后再删除目录；实验失败以非零退出码汇总。
退出码为 `0` 全部通过或列表/帮助成功，`1` 实验、构建或清理失败，`2` 参数错误。
`context-stream` 另外检查流 draft 修改、替换、丢弃、拆分、重复设置、异步数组增长和取消后迟到结果；
全部 18 个 VM 实验（包含 `environment` 与 `context-stream`）已通过 Linux CI 与 Windows 隔离运行验证。
启动实验在成功返回后立即检查 `Ready`；`memory-limit` 另外覆盖同步抛出或 Promise rejection 携带
`null` / 自身 `message: "out of memory"` 的保守退役边界；这些场景已通过 Windows 与 Linux CI 隔离运行。

`environment` 通过父进程的子进程启动配置注入合成变量，不修改多线程进程的全局环境，
也不需要 `unsafe`。它覆盖顶层、本地 ESM、异步 hook 的密钥读取、缺失与空值、Unicode、
非法参数、非 Unicode 值、名称/值字节上限、只读绑定、导入前命名空间冻结和失败后恢复；
纯逻辑单元测试独立检查校验与错误来源。
该实验与完整 18 项 VM 实验均已通过 Windows 与 Linux CI 隔离运行验证。

检查示例配置：

```sh
cargo run --locked -- check --config examples/basic/server.toml
```

`check` 会执行顶层 JS，但不执行请求阶段 hook。CLI 退出码与操作边界见[功能指南](features.md#命令行与配置检查)。
运行时库只报告退役；HTTP 服务监听状态变更，停止监听、取消在途请求并非零退出，
不继续接收依赖失效 VM 的请求，也不自动重放上游请求。

## 本地 HTTP 实验

单独运行本地合成上游与网关，每个实验在有外部超时保护的 Rust worker 中执行。
驱动只绑定回环地址并使用临时合成配置和占位秘密；不连接真实 provider。

```sh
cargo xtask probe http
cargo xtask probe http transport-contracts
cargo xtask probe http request-overrides pipeline stream-transforms
cargo xtask probe http --executable target/debug/limen
```

`--list` 查看实验，位置参数选择单项，`--executable PATH` 复用已构建二进制，`--timeout` 设置单项外部上限。
HTTP 外部上限默认 18 秒，接受 5–60 秒。未提供 `--executable` 时通过 Cargo 构建 `limen`，
从 JSON artifact 消息读取可执行文件位置，支持自定义 target-dir 与含空格的路径。
Unix 使用独立进程 session，Windows 使用 Job Object；超时、失败及父进程中断均清理整棵进程树。
VM 与 HTTP 的临时根都由父进程持有；HTTP worker 在其中创建 fixture 子目录。
合成上游复用 Hyper，客户端复用 reqwest；客户端断开实验另外持有原始 TCP socket。
实验覆盖认证/路由、非流管线、失败状态与媒体类型、资源上限、观察预算、deadline、断开、关闭和 VM 退役。
`request-overrides` 检查上游实际收到的参数、URL 和请求头，覆盖 provider/model/client/hook 优先级、
对象与数组的浅层替换、显式 null、字段删除、多模态/工具/扩展字段保留，以及改写 model 后不重新路由。
`pipeline` 以客户端最终文本、状态码、响应头和上游请求检查异步响应改写、请求状态隔离与失败后恢复；
异常、Promise rejection、非 void 完成值和循环 draft 都必须返回安全错误且不发送上游请求。
`stream-transforms` 检查 SSE 丢弃与拆分后的实际片段、聚合文本、usage 和对外模型名，
也覆盖完整 JSON 响应改写后转换为 SSE。上述断言不依赖内部调用轨迹或执行计数。
`transport-contracts` 覆盖四种传输组合、原始客户端流意图、usage 尾块与截断后的流内错误，
并检查首个 DONE 后的错误事件位于同一块或延迟尾块时，JSON/SSE 下游均保持成功终结。
该终结回归已在 Windows 与 Linux CI 通过；Windows 负向对照中临时恢复旧的块尾终结逻辑时，实验会拒绝其错误结果。
全部 12 个 Rust HTTP 实验已通过 Windows 和 Linux CI 隔离运行验证，覆盖参数覆写、流变换、管线与终结分块一致性。
Linux 的 149 项单元测试、18 个 VM 实验与 12 个 HTTP 实验均通过，证据见
[`c562fa0` 的 CI Rust 任务](https://github.com/rhnu/limen/actions/runs/38674755676/job/116225705564)。
Windows 关闭实验由两个短命 helper 显式脱离继承控制台，再创建或连接网关私有控制台，隔离 Ctrl-C 广播。
正常实验清理与独立控制台 Ctrl-C 已有运行证据；Job Object 在外部超时和父进程中断时的回收仍待单独验证。
纯逻辑测试不启动 HTTP 或 VM；JS 与 HTTP 驱动作为独立高级测试执行，CI 中固定排在单元测试之后。
日志检查使用合成秘密验证正文、令牌、Cookie、源码异常文本和 query 不出现在应用日志中。

## 当前依赖选择

引入时已查询 crates.io 元数据、上游发布或标签及实际 feature/source；锁文件记录真实解析结果。
以下版本不构成自动升级策略，升级时重新核对注册表、发布记录、features、MSRV 和平台要求。

| 依赖                | 解析版本 | MSRV | 启用能力与依据                                                                 |
| ------------------- | -------- | ---- | ------------------------------------------------------------------------------ |
| axum                | 0.8.9    | 1.80 | 禁用默认 features；body/response 类型与有界正文读取                            |
| hyper               | 1.12.0   | 1.63 | http1、server；连接由应用持有，断开与关闭可传播到请求                          |
| hyper-util          | 0.1.21   | 1.85 | tokio；TokioIo、TokioTimer                                                     |
| http-body-util      | 0.1.5    | 1.61 | 有类型的正文长度限制错误                                                       |
| reqwest             | 0.13.5   | 1.85 | rustls；应用负责有界 JSON 编码，禁用默认 features、自动解压、HTTP/2 和环境代理 |
| clap                | 4.6.7    | 1.85 | std、derive、help、usage；禁用默认 features，避免参数错误上下文回显输入        |
| rquickjs            | 0.14.0   | 1.87 | std、futures、loader；不启用 parallel、rust-alloc；保留 QuickJS 内建分配统计   |
| Tokio               | 1.53.3   | 1.71 | rt、sync、time、macros、net、signal、io-util；单线程 executor、HTTP 与取消     |
| serde               | 1.0.229  | 1.56 | derive，强类型配置                                                             |
| serde_json          | 1.0.154  | 1.71 | 默认 std，不启用无限深度或任意精度来掩盖 JS 数字边界                           |
| serde-saphyr        | 2.0.0    | 1.89 | 禁用默认 features，仅 deserialize；配置解析预算和位置                          |
| toml                | 1.1.8    | 1.85 | 禁用默认 features，仅 std、serde、parse；共用 schema，保留位置和递归深度保护   |
| serde_path_to_error | 0.1.20   | 1.61 | 保留字段路径，输出前隐藏任意用户键名                                           |
| url                 | 2.5.8    | 1.63 | URL 结构校验；额外禁止 userinfo、fragment、反斜杠与空白                        |
| http                | 1.5.0    | 1.57 | 成熟的 header 名称和值解析                                                     |
| tracing             | 0.1.44   | 1.65 | std，无属性宏；应用元数据日志                                                  |
| tracing-subscriber  | 0.3.23   | 1.65 | fmt、json，无环境过滤器；stderr 输出                                           |
| thiserror           | 2.0.21   | 1.77 | std；库和应用错误保留类型、上下文与 source 链                                  |
| sse-parse           | 0.1.0    | 1.65 | 默认 std；sans-IO 增量解析，项目逐字节推进并将 overflow 转为明确失败           |
| anyhow              | 1.0.104  | 1.68 | xtask 应用边界错误与保留来源的上下文                                           |
| tempfile            | 3.27.0   | 1.63 | xtask 父进程持有的临时目录与显式清理                                           |
| process-wrap        | 10.0.1   | 1.87 | std/Tokio 进程封装、Unix session、Windows Job Object 与创建标志                |
| windows-sys         | 0.61.2   | 1.71 | 仅 xtask Windows 控制台原生调用                                                |

xtask 新依赖核对了 crates.io 最新稳定版、MSRV 和上游标签/源码：
[anyhow 发布](https://github.com/dtolnay/anyhow/releases/tag/1.0.104)、
[tempfile 标签](https://github.com/Stebalien/tempfile/tree/v3.27.0)、
[process-wrap 标签](https://github.com/watchexec/process-wrap/tree/v10.0.1)与
[windows-sys 发布清单](https://docs.rs/crate/windows-sys/0.61.2/source/Cargo.toml)。
process-wrap 的创建标志使用其 shim，与 Job Object 的 suspended 启动及赋值顺序保持一致。
Tokio 的 `process` feature 仅由 xtask 请求；新依赖不加入根应用的直接依赖。

上游依据：[clap 发布](https://github.com/clap-rs/clap/releases/tag/v4.6.7)与
[feature 文档](https://docs.rs/crate/clap/4.6.7/features)、[rquickjs 文档](https://docs.rs/rquickjs/0.14.0/rquickjs/)、
[Tokio 发布](https://github.com/tokio-rs/tokio/releases)、[Serde 发布](https://github.com/serde-rs/serde/releases)、
[serde_json 发布](https://github.com/serde-rs/json/releases)、[serde-saphyr 发布](https://github.com/bourumir-wyngs/serde-saphyr/releases)、
[字段路径发布](https://github.com/dtolnay/path-to-error/releases)、[url 发布](https://github.com/servo/rust-url/releases)、
[http 发布](https://github.com/hyperium/http/releases)、[tracing 发布](https://github.com/tokio-rs/tracing/releases)、
[thiserror 注册表](https://crates.io/crates/thiserror/2.0.21)与[上游清单](https://github.com/dtolnay/thiserror/blob/2.0.21/Cargo.toml)。

HTTP 引入时重新核对了注册表及上游发布/feature：
[Axum 发布](https://github.com/tokio-rs/axum/releases/tag/axum-v0.8.9)、
[Hyper 发布](https://github.com/hyperium/hyper/releases/tag/v1.12.0)、
[hyper-util 发布](https://github.com/hyperium/hyper-util/releases/tag/v0.1.21)、
[http-body-util 发布](https://github.com/hyperium/http-body/releases/tag/http-body-util-v0.1.5)、
[reqwest 发布](https://github.com/seanmonstar/reqwest/releases/tag/v0.13.5)与
[reqwest features](https://docs.rs/crate/reqwest/0.13.5/features)。
reqwest 0.13 的 `rustls` feature 使用平台证书验证与 AWS-LC；Windows 本机构建已验证，CMake 可用，
Linux CI 与镜像构建已通过 C 编译；运行镜像包含系统 CA 证书。

TOML 依赖核对了 [crates.io 注册表](https://crates.io/crates/toml/1.1.8+spec-1.1.0)、
[上游发布记录](https://github.com/toml-rs/toml/blob/toml-v1.1.8/crates/toml/CHANGELOG.md)及发布清单。
MSRV 1.85 兼容固定的 Rust 1.99.0，不启用 display、preserve_order、unbounded 或平台专用能力。
服务配置按扩展名解析，共享 schema 和校验；默认选择及错误语义见[设计](design.md#31-服务配置进程与资源边界)。

SSE 解码对照 WHATWG，并调查了 eventsource-stream、sse-codec 与 sse-stream；选用可独立于异步网络层使用的
[sse-parse 发布](https://github.com/hey-jj/sse-parse/releases/tag/v0.1.0)与
[发布清单](https://github.com/hey-jj/sse-parse/blob/v0.1.0/Cargo.toml)。
`limen-chat` 在其上增加输入事件预算、Chat 语义与结束校验，避免依赖丢弃超限块后继续运行掩盖截断。
JSON→SSE 使用有界迭代，不缓存整段生成事件；SSE→JSON 聚合累计输入大小和事件数，跨形态未知结构明确拒绝。

## 错误与日志

配置错误保留 `ConfigErrorKind`、schema 字段及可用位置；脚本错误通过 `ScriptErrorKind` 分类，
`ScriptError::kind()` 用于处理决策。`RuntimeStatus` 包含完整退役错误，使用模式匹配检查状态。
错误通过 `thiserror` 的 `source`/`from` 或保留来源的转换传播；脚本错误手动实现 `source()`，
直接返回共享原因的原始类型，避免 `Arc` 包装妨碍 downcast。主错误与清理失败分别保存。
纯逻辑单元测试检查分类、来源类型、上下文及安全诊断，不执行 VM。
JS 异常诊断另外保存固定内建类别、模块编号和可用行列，重分类与克隆不丢失这些信息，原始引擎错误仍保留为 source。
`error/exception` 的纯逻辑测试验证栈位置匹配与安全输出；`rejection` 隔离 VM 实验验证启动语法错误、
顶层 await、导入模块及同步/异步 hook 异常，以及 getter、Proxy、内建函数覆写和自定义栈下的诊断与失败后恢复。
这些诊断场景纳入现有 `rejection` 实验，由本机检查与 Linux CI 运行。

原始 `source()` 可能含配置、键名或源码，仅用于内部类型检查；CLI、日志与 Debug 使用安全诊断。
库的 `diagnostic()` 输出有界原因分类，应用补充失败阶段和逻辑配置文件类别，避免直接格式化任意来源。
入口模块使用固定标签 `config.js`，导入模块使用按首次加载顺序分配的 `import#N`；日志不输出实际模块路径、函数名或栈文本。
诊断桥接在用户模块运行前捕获 QuickJS 内建栈 getter，直接读取 Error 的内部栈数据，绕过用户定义的访问器；
只有不超过 4096 个 UTF-16 code unit 且 Unicode 有效的栈字符串进入 VM 线程内的位置解析器。
解析器最多处理 16384 字节与 64 帧，仅匹配已登记模块并输出正整数行列；未知位置省略，异常 message 不提取。
应用负责 subscriber 与最终失败报告；库仅建立操作 span，异步和线程入口显式传播上下文。
默认等级为 info，格式为 text，输出为 stderr；日志配置被拒绝时不静默回退。
预期配置拒绝为 warn，操作失败为 error；过滤不能让 CLI 失去失败原因。panic payload 不进入输出。

## Git 与配置文件

- 默认分支为 `main`；保持仓库 LF 换行约定，不修改全局 Git 身份或配置。
- 提交遵循 Conventional Commits，描述具体行为或职责变化，不使用路线图阶段编号作为提交主题。
- 保留 `Cargo.lock`。提交前检查暂存区，确保无真实配置、令牌、Cookie、携密源码或原始请求正文。
- 本机真实配置集中在 `local/`，Docker 部署配置放 `data/`；`examples/` 只放占位值。Git ignore 不能保证其他路径没有秘密。
- 项目许可证为 `AGPL-3.0-only`，完整条款来自 [GNU 官方文本](https://www.gnu.org/licenses/agpl-3.0.txt)，保存于根目录 [LICENSE](../LICENSE)。
  `workspace.package.license` 集中声明许可证，根包、职责库和 xtask 均通过 `license.workspace = true` 继承，Cargo 包继续设置为不发布。
- 不从模板推断 GitHub owner、公开性或镜像命名空间；发布前另行确定。

## GitHub CI 与容器构建

CI 在 `main` push、pull request 和手动触发时运行，使用 `ubuntu-24.04`，默认只授予 `contents: read`，
镜像 job 另授予 `packages: write`。同 ref 运行串行排队，`queue: max` 最多容纳 100 个等待运行，
超过 GitHub 队列上限的运行会被取消；新运行不取消正在执行的构建与发布。每个 job 有超时；checkout 不持久化凭据。
Rust job 使用工具链与锁文件相关的缓存，依次编译检查、格式检查、lint、纯逻辑单元测试、
`cargo xtask probe runtime` 与 `cargo xtask probe http`；Markdown job 检查仓库文档。
两组高级测试各自使用 worker 进程与单项外部超时，任一失败都会阻止后续镜像构建与发布。

GitHub 远程为 `rhnu/limen`，Linux CI 与容器镜像构建已跑通，分支保护可依据现有 CI 配置。
镜像 job 等待 Rust 与 Markdown 检查通过，使用 Buildx 构建 `linux/amd64` 和 GHA `mode=max` 中间层缓存。
仅 `main` push 使用 `GITHUB_TOKEN` 登录 GHCR 并推送 `ghcr.io/<小写 owner>/<小写 repo>:latest`，
当前仓库为 `ghcr.io/rhnu/limen:latest`。PR 和手动触发使用 `cacheonly` 完成构建，不登录、不推送、不清理。
CI 不运行容器。Dockerfile 使用 cargo-chef 依赖层与独立源码构建层，减少只改应用源码时的依赖重编译；
Rust 1.99.0 slim trixie 构建，Debian trixie slim 运行。运行层包含二进制、入口脚本、CA 与运行依赖；
入口以 root 修正权限后降权执行应用。
release 启用 thin LTO、单 codegen unit 和去除调试信息，保留 unwind 以维持运行时的异常清理边界。
Dockerfile 声明 `org.opencontainers.image.licenses=AGPL-3.0-only`，运行层保留
`/usr/share/doc/limen/LICENSE`；构建上下文允许清单包含根目录许可证。

[`c562fa0` 的完整 CI](https://github.com/rhnu/limen/actions/runs/38674755676) 已通过 Rust、Markdown 和镜像任务，
包括 Linux amd64 镜像构建、GHCR latest 发布与旧版本清理。

成功推送后，使用 GitHub Packages REST API 分页获取版本，确认 `latest` 的 digest 与本次 Buildx 输出一致，
再删除该包的所有其他版本（包括无标签和带其他标签的历史版本）。重复推送同一 digest 时仍保留实际 `latest`，
不按版本创建时间判断。清理失败使 job 失败，已成功推送的 latest 保留；构建或推送失败不执行清理。
只发布单架构单 manifest，显式关闭 provenance 与 SBOM attestations，避免清理无标签版本时删除 latest 引用的子 manifest。
未来启用多架构或 attestations 时需先调整清理策略以保留完整引用图。

workflow 发布的包自动关联仓库，并带 `org.opencontainers.image.source` 和 revision 标签。
无需额外 PAT；删除版本要求该仓库对包拥有 admin 权限。如果同名包曾由其他方式创建，需在包设置中关联仓库、
授予 Actions 访问权限。新包默认 private，可按需要在 GHCR 包设置中调整可见性。
公开包版本超过 GitHub 的 5,000 次下载删除限制时，清理会失败并需 GitHub 支持处理。
权限、可见性与删除限制见 [GHCR 官方说明](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)、
[Packages REST API](https://docs.github.com/en/rest/packages/packages)；队列行为见
[Actions concurrency](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)。

Compose 使用已发布的 `ghcr.io/rhnu/limen:latest`。首次将 `examples/docker/` 复制为 `data/`，
填写 `data/server.toml` 和 `data/config.js`，再执行：

```sh
docker compose config
docker compose pull
docker compose run --rm limen check
docker compose up -d
docker compose down
```

需要从源码构建镜像时单独执行 `docker build --platform linux/amd64 -t limen:local .`；Compose 没有本地构建段。
镜像默认工作目录或启动命令调整后，部署环境需先等待 CI 发布更新后的镜像，再执行 `docker compose pull`。

Compose 用 `./data:/data` 挂载部署目录，容器工作目录为 `/data`；`CMD ["serve"]` 与检查命令均优先读取 `server.toml`，
仅缺失时尝试 `server.yaml`，不合并配置。
Docker 示例独立提供 `server.toml`、等价的 `server.yaml` 与 `config.js`，无需复制本机示例或共享规则；自行导入的模块需一同放入 `data/`。
首次准备使用 `mkdir -p data` 和 `cp -R examples/docker/. data/`，将文件直接放到部署目录根部。
短挂载语法在宿主目录缺失时会创建空目录，不会生成配置文件；启动前必须完成示例复制和配置填写。
`docker-entrypoint.sh` 在 root 启动路径执行 `chgrp -Rh 10001 /data` 和 `chmod -R g=rX /data`，
保留文件属主，不跟随树内符号链接修改外部目标；组仅保留读取与目录访问权限。
随后用 Debian `util-linux` 的 `setpriv` 清空附加组、继承和 ambient capabilities，设置 `no_new_privs`，
通过 `exec` 以 UID/GID 10001 运行 Limen，保留 PID 1 和信号传播。任一步骤失败即退出，不继续以 root 启动应用。
显式指定非 root `--user` 时直接执行应用，由调用者确保配置可读；正常启动、检查与重启均经过入口。
`util-linux` 来自运行镜像相同的 trixie 仓库；核对了 [Debian 包与文件清单](https://packages.debian.org/trixie/amd64/util-linux/filelist)
和 [setpriv 手册](https://manpages.debian.org/trixie/util-linux/setpriv.1.en.html)，不额外下载降权程序。
Compose 移除其余 capabilities，仅保留权限初始化与降权所需的 CHOWN、DAC_OVERRIDE、FOWNER、SETUID、SETGID。
容器内 `0.0.0.0:8080`，宿主默认仅绑定回环地址；根文件系统只读，部署目录读写挂载以修改权限，应用不需要写配置。
终止宽限时间覆盖 HTTP 与 VM 的有限关闭预算。`.dockerignore` 使用允许清单控制构建上下文，
不拷贝 `local/`、`data/`、示例、Git 历史或任意未列入的文件。
Compose 的 512 MiB 内存与 128 PID 限制是示例运行上界，不是容量或吞吐承诺；
入口为 UID/GID 10001 准备读取权限。Windows NTFS/共享权限、rootless/userns-remap 的
宿主 ID 映射和 SELinux 标签仍由部署环境提供；不支持修改属组或模式的挂载会使初始化失败，不静默跳过。
部署时用 `docker compose run --rm limen check` 检查真实挂载下的读取和配置装载。
挂载和 `security_opt` 的规则见 [Compose 官方服务配置参考](https://docs.docker.com/reference/compose-file/services/)，
宿主约束见 [bind mounts](https://docs.docker.com/engine/storage/bind-mounts/) 和
[用户命名空间映射](https://docs.docker.com/engine/security/userns-remap/)。

构建工具 cargo-chef 固定为 0.1.78，版本来源为
[官方发布](https://github.com/LukeMathWalker/cargo-chef/releases/tag/v0.1.78)。
缓存与镜像构建使用 [Rust cache](https://github.com/Swatinem/rust-cache)、
[Buildx action](https://github.com/docker/setup-buildx-action)和
[Build push action](https://github.com/docker/build-push-action)。
GHCR 登录使用 [Docker login action v4](https://github.com/docker/login-action)，清理使用
[GitHub Script v9](https://github.com/actions/github-script) 提供的 Octokit 分页和 REST 请求；两者使用 Node 24，适配 GitHub 托管 runner。
