# 开发指引

## 目录与依赖方向

```text
.
├── apps/limen/          CLI、共享 bootstrap、日志与显式运行时实验入口
├── modules/
│   ├── limen-config/    YAML/业务配置、资源 schema、错误与状态契约
│   └── limen-script/    ESM、严格 JSON 桥接、专属线程和有界 JS 调度
├── examples/            无真实凭据的基础配置与共享模块
├── tools/               有外部超时保护的运行时实验驱动
├── docs/                设计、唯一实施路线图、开发指引
├── Cargo.toml           workspace、集中依赖和共享 lint
├── Cargo.lock           应用依赖解析结果，纳入版本控制
├── rust-toolchain.toml  固定 Rust 1.99.0、rustfmt 与 Clippy
└── rusteward.toml       Rusteward 格式与 lint 配置
```

应用组合配置和脚本库；配置库不依赖 VM，脚本库不依赖 HTTP 或配置 schema。
跨线程仅传 owned JSON、ID、deadline 与取消信号，库的公开 API 不暴露 rquickjs 句柄。
后续在实际需求出现时增加 pipeline、http/upstream、protocol 模块，不预建空 crate。
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
`--lib --bins` 只运行配置、JSON 输入边界和 CLI 等纯逻辑单元测试，不隐式运行集成或文档测试。

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

检查示例配置：

```sh
cargo run -p limen --locked -- check --config examples/basic/server.yaml
```

`check` 会执行顶层 JS，但不执行请求阶段 hook。CLI 退出码与操作边界见 README。
运行时库只报告退役，应用负责在启动/关闭失败时非零退出；后续 HTTP 服务必须监听退役状态并结束进程，
不能继续接收依赖失效 VM 的请求，也不能自动重放上游请求。

## 当前依赖选择

引入时已查询 crates.io 元数据、上游发布或标签及实际 feature/source；锁文件记录真实解析结果。
以下版本不构成自动升级策略，升级时重新核对注册表、发布记录、features、MSRV 和平台要求。

| 依赖                | 解析版本 | MSRV | 启用能力与依据                                                               |
| ------------------- | -------- | ---- | ---------------------------------------------------------------------------- |
| rquickjs            | 0.14.0   | 1.87 | std、futures、loader；不启用 parallel、rust-alloc；保留 QuickJS 内建分配统计 |
| Tokio               | 1.53.3   | 1.71 | rt、sync、time、macros；专属 current-thread executor 与有界通道              |
| serde               | 1.0.229  | 1.56 | derive，强类型配置                                                           |
| serde_json          | 1.0.154  | 1.71 | 默认 std，不启用无限深度或任意精度来掩盖 JS 数字边界                         |
| serde-saphyr        | 2.0.0    | 1.89 | 禁用默认 features，仅 deserialize；配置解析预算和位置                        |
| serde_path_to_error | 0.1.20   | 1.61 | 保留字段路径，输出前隐藏任意用户键名                                         |
| url                 | 2.5.8    | 1.63 | URL 结构校验；额外禁止 userinfo、fragment、反斜杠与空白                      |
| http                | 1.5.0    | 1.57 | 成熟的 header 名称和值解析                                                   |
| tracing             | 0.1.44   | 1.65 | std，无属性宏；应用元数据日志                                                |
| tracing-subscriber  | 0.3.23   | 1.65 | fmt、json，无环境过滤器；stderr 输出                                         |

上游依据：[rquickjs 文档](https://docs.rs/rquickjs/0.14.0/rquickjs/)、
[Tokio 发布](https://github.com/tokio-rs/tokio/releases)、[Serde 发布](https://github.com/serde-rs/serde/releases)、
[serde_json 发布](https://github.com/serde-rs/json/releases)、[serde-saphyr 发布](https://github.com/bourumir-wyngs/serde-saphyr/releases)、
[字段路径发布](https://github.com/dtolnay/path-to-error/releases)、[url 发布](https://github.com/servo/rust-url/releases)、
[http 发布](https://github.com/hyperium/http/releases)、[tracing 发布](https://github.com/tokio-rs/tracing/releases)。

## Git 与配置文件

- 默认分支为 `main`；保持仓库 LF 换行约定，不修改全局 Git 身份或配置。
- 提交遵循 Conventional Commits，描述具体行为或职责变化，不使用路线图阶段编号作为提交主题。
- 保留 `Cargo.lock`。提交前检查暂存区，确保无真实配置、令牌、Cookie、携密源码或原始请求正文。
- 实际部署配置集中在 `local/`；`examples/` 只放占位值。Git ignore 不能保证其他路径没有秘密。
- 不从模板推断许可证、GitHub owner、公开性或镜像命名空间；发布前另行确定。

## GitHub CI 与后续部署

CI 在 `main` push、pull request 和手动触发时运行，使用 `ubuntu-24.04`，只授予 `contents: read`。
同 ref 新运行取消旧运行；每个 job 有超时；checkout 不持久化凭据。
Rust job 依次编译检查、格式检查、lint、纯逻辑单元测试；Markdown job 检查仓库文档。
隔离运行实验当前手动调用，不自动加入普通 CI 测试步骤。

尚未配置远程或运行托管 runner。建立远程后运行一次 CI，再依据实际结果配置分支保护。
Dockerfile、容器运行检查与镜像发布仍按路线图部署阶段安排；本机成功不代表目标平台通过。
