# 开发指引

## 目录与依赖方向

```text
.
├── src/                 根包：CLI、bootstrap、JSON/SSE gateway、HTTP server、日志与运行时实验入口
├── modules/
│   ├── config/          limen-config：YAML/业务配置、资源 schema、配置诊断
│   ├── script/          limen-script：ESM、严格 JSON 桥接、专属线程和有界 JS 调度
│   └── chat/            limen-chat：SSE 编解码、chunk 校验、四路径转换与有界聚合
├── examples/            无真实凭据的基础配置与共享模块
├── tools/               有外部超时保护的 JS 与本地 HTTP 实验驱动
├── docs/                设计、唯一实施路线图、开发指引
├── Cargo.toml           根包、workspace、集中依赖和共享 lint
├── Cargo.lock           应用依赖解析结果，纳入版本控制
├── rust-toolchain.toml  固定 Rust 1.99.0、rustfmt 与 Clippy
└── rusteward.toml       Rusteward 格式与 lint 配置
```

应用组合配置、脚本和 Chat 协议库；配置库不依赖 VM，脚本库不依赖 HTTP 服务框架或配置 schema（使用 http 校验头部）。
Chat 库仅消费 owned 字节与 JSON，不持有连接，也不依赖配置或 VM。
根包 `limen` 是默认 workspace 成员，根目录的 `cargo run`、`cargo build` 和 `cargo test` 默认选择它；
全 workspace 检查与测试继续显式使用 `--workspace`。`runtime_probe` 同属根包，仅由隔离实验驱动运行。
跨线程仅传 owned JSON、ID、deadline 与取消信号，库的公开 API 不暴露 rquickjs 句柄。
根包 gateway 模块负责路由、管线与错误，server 模块拥有连接、关闭和 VM 退役传播。
短目录名表示职责，Cargo 包名使用 `limen-` 前缀；路径依赖由根清单显式指定。
公开错误码、状态映射和安全错误对象属于根包 gateway；职责见
[设计中的工程边界](design.md#12-workspace-与模块边界)。
单元测试放在被测模块的 `tests.rs` 子模块，禁止 `mod.rs`。

## 本地检查

QuickJS 包含 C 源码，需要可用的 C 编译工具链。Windows 本机已能构建其 MSVC 目标；
Linux 需准备 C compiler、归档器及 libc 开发环境，目标平台实测仍列于路线图。

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
rumdl check --fix README.md docs/design.md docs/roadmap.md docs/development.md
```

本机使用 rumdl 0.2.79，CI 同版本。可用时另运行 `actionlint .github/workflows/ci.yml`，并执行 `git diff --check`。

## 隔离运行时实验

JS 执行实验与普通单元测试分开。Python 3 标准库驱动构建专用二进制，再将每个实验放入独立子进程，
超过外部时间上限即终止，避免同步死循环或 VM 故障让单元测试及 CI 无界挂起。

```sh
python tools/runtime-probes.py
```

使用 `python tools/runtime-probes.py --help` 查看单项实验、超时和列表选项。
不要直接运行失控脚本实验二进制而省略外部超时保护。
实验只使用临时合成配置；不会连接真实 provider，不属于 Linux、Docker 或 HTTP 端到端验收。
临时目录由 Python 父进程统一持有和清理，覆盖正常完成、失败与外部超时终止；Rust 实验入口只负责写入合成配置。
`context-stream` 另外检查流 draft 修改、替换、丢弃、拆分、重复设置、异步数组增长和取消后迟到结果；
该实验需单独调用，运行证据仍待补齐。

检查示例配置：

```sh
cargo run --locked -- check --config examples/basic/server.yaml
```

`check` 会执行顶层 JS，但不执行请求阶段 hook。CLI 退出码与操作边界见 README。
运行时库只报告退役；HTTP 服务监听状态变更，停止监听、取消在途请求并非零退出，
不继续接收依赖失效 VM 的请求，也不自动重放上游请求。

## 本地 HTTP 实验

单独运行本地合成上游与网关，每个实验在有外部超时保护的 Python 子进程中执行。
驱动只绑定回环地址并使用临时合成配置和占位秘密；不连接真实 provider。

```sh
python tools/http-probes.py
```

`--list` 查看实验，位置参数选择单项，`--executable PATH` 复用已构建二进制，`--timeout` 设置单项外部上限。
实验覆盖认证/路由、非流管线、失败状态与媒体类型、资源上限、观察预算、deadline、断开、关闭和 VM 退役。
`transport-contracts` 覆盖四种传输组合、原始客户端流意图、usage 尾块与截断后的流内错误，运行证据仍待补齐。
Windows 关闭实验为独立隐藏 console 下的 Ctrl-C，Unix 为 SIGTERM；当前执行证据只覆盖 Windows。
纯逻辑测试不启动 HTTP 或 VM；JS 与 HTTP 驱动均显式调用，不混入普通单元测试或自动 CI。
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
| serde_path_to_error | 0.1.20   | 1.61 | 保留字段路径，输出前隐藏任意用户键名                                           |
| url                 | 2.5.8    | 1.63 | URL 结构校验；额外禁止 userinfo、fragment、反斜杠与空白                        |
| http                | 1.5.0    | 1.57 | 成熟的 header 名称和值解析                                                     |
| tracing             | 0.1.44   | 1.65 | std，无属性宏；应用元数据日志                                                  |
| tracing-subscriber  | 0.3.23   | 1.65 | fmt、json，无环境过滤器；stderr 输出                                           |
| thiserror           | 2.0.21   | 1.77 | std；库和应用错误保留类型、上下文与 source 链                                  |
| sse-parse           | 0.1.0    | 1.65 | 默认 std；sans-IO 增量解析，项目逐字节推进并将 overflow 转为明确失败           |

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
Linux 的 C 构建、证书与运行环境仍须部署阶段单独验证。锁文件不代表已执行目标平台或 TLS 真实上游验收。

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

原始 `source()` 可能含配置、键名或源码，仅用于内部类型检查；CLI、日志与 Debug 使用安全诊断。
库的 `diagnostic()` 输出有界原因分类，应用补充失败阶段和逻辑配置文件类别，避免直接格式化任意来源。
应用负责 subscriber 与最终失败报告；库仅建立操作 span，异步和线程入口显式传播上下文。
默认等级为 info，格式为 text，输出为 stderr；日志配置被拒绝时不静默回退。
预期配置拒绝为 warn，操作失败为 error；过滤不能让 CLI 失去失败原因。panic payload 不进入输出。

## Git 与配置文件

- 默认分支为 `main`；保持仓库 LF 换行约定，不修改全局 Git 身份或配置。
- 提交遵循 Conventional Commits，描述具体行为或职责变化，不使用路线图阶段编号作为提交主题。
- 保留 `Cargo.lock`。提交前检查暂存区，确保无真实配置、令牌、Cookie、携密源码或原始请求正文。
- 实际部署配置集中在 `local/`；`examples/` 只放占位值。Git ignore 不能保证其他路径没有秘密。
- 不从模板推断许可证、GitHub owner、公开性或镜像命名空间；发布前另行确定。

## GitHub CI 与容器构建

CI 在 `main` push、pull request 和手动触发时运行，使用 `ubuntu-24.04`，只授予 `contents: read`。
同 ref 新运行取消旧运行；每个 job 有超时；checkout 不持久化凭据。
Rust job 使用工具链与锁文件相关的缓存，依次编译检查、格式检查、lint、纯逻辑单元测试；Markdown job 检查仓库文档。
隔离运行实验当前手动调用，不自动加入普通 CI 测试步骤。

尚未配置远程或运行托管 runner。建立远程后运行一次 CI，再依据实际结果配置分支保护。
镜像 job 通过 Buildx 构建 `linux/amd64`，使用 GHA `mode=max` 缓存中间层，不登录 registry、不推送镜像，
也不运行容器。Dockerfile 使用 cargo-chef 依赖层与独立源码构建层，减少只改应用源码时的依赖重编译；
Rust 1.99.0 slim trixie 构建，Debian trixie slim 运行。运行层只含二进制、CA 与运行依赖，使用非 root 用户。
release 启用 thin LTO、单 codegen unit 和去除调试信息，保留 unwind 以维持运行时的异常清理边界。

本地可显式调用以下命令（本次未运行）：

```sh
docker build --platform linux/amd64 -t limen:local .
docker compose run --rm limen check --config /config/docker/server.yaml
docker compose up --build -d
docker compose down
```

Compose 只读挂载整个 `local/`，配置使用 `/config/docker/server.yaml`，示例重导出 `../basic/config.js`，
继续使用 `../shared/`。复制整个 `examples/` 到 `local/` 后填写凭据，保持目录关系。
容器内 `0.0.0.0:8080`，宿主默认仅绑定回环地址；只读根文件系统、移除 capabilities，禁止提权。
终止宽限时间覆盖 HTTP 与 VM 的有限关闭预算。`.dockerignore` 使用允许清单控制构建上下文，
不拷贝 `local/`、示例、Git 历史或任意未列入的文件。
Compose 的 512 MiB 内存与 128 PID 限制是示例运行上界，不是容量或吞吐承诺；
配置需允许 UID/GID 10001 读取，尤其留意宿主上仅所有者可读的凭据文件。

构建工具 cargo-chef 固定为 0.1.78，版本来源为
[官方发布](https://github.com/LukeMathWalker/cargo-chef/releases/tag/v0.1.78)。
缓存与镜像构建使用 [Rust cache](https://github.com/Swatinem/rust-cache)、
[Buildx action](https://github.com/docker/setup-buildx-action)和
[Build push action](https://github.com/docker/build-push-action)。
容器运行、Linux 构建与 TLS/CA、SIGTERM、慢客户端及流背压仍待独立验收；镜像推送和 release 需先明确发布位置与权限。
