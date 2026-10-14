# 开发指引

## 当前目录与拟定模块

```text
.
├── .github/             CI、依赖更新、PR 模板
├── docs/                设计、路线图、开发指引
├── apps/limen/          应用 crate；目前只有未实现状态入口
├── Cargo.toml           workspace 与共享约定
├── Cargo.lock           应用解析锁文件，纳入版本控制
├── rust-toolchain.toml  固定工具链
└── rusteward.toml       项目格式与 lint 配置
```

先在应用 crate 内按职责形成模块；下表是计划，不预建空模块或空 crate。

| 模块                | 职责与边界                                           |
| ------------------- | ---------------------------------------------------- |
| `cli` / `config`    | 参数、基础配置、共享加载流程与结构校验               |
| `script`            | 专属线程、ESM、hook 注册、owned 命令与结果、job pump |
| `pipeline`          | 阶段推进、冻结路由、draft 提交、取消与终态           |
| `http` / `upstream` | 入口认证、资源限制、模型表、一次上游请求             |
| `protocol`          | JSON/SSE 语义、增量解码、聚合和编码                  |
| `telemetry`         | 清理后的日志元数据与统计                             |

依赖方向为入口组合配置、管线和传输适配；协议与路由纯逻辑不依赖 HTTP handler。
script 边界不向其他模块暴露 `rquickjs` 句柄。若编译隔离或复用需求成立，再将 script/protocol 提取成 workspace 成员。
多个成员出现重复开发编排需求时再引入 `xtask`，当前直接使用工具命令。

## 本地检查

在仓库根目录按顺序执行：

```sh
cargo check --workspace --all-targets --locked
cargo dev format --locked
cargo dev lint --locked --deny-warnings
cargo test --workspace --bins --locked
rumdl check --fix README.md AGENTS.md docs/design.md docs/roadmap.md docs/development.md .github/pull_request_template.md
```

`cargo dev format` 会修改源文件；只读检查使用 `cargo dev format --check --diff --locked`。
Rusteward 配置由 `rusteward.toml` 统一维护。修改 Markdown 后仅将本次变更文件传给 rumdl；
上面的完整列表适用于初始化，CI 则执行整个仓库的只读 Markdown 检查。

当前只有 binary target，所以单元测试命令使用 `--bins`；新增 library target 时同步改为 `--lib --bins`。
不要直接改成无筛选的 `cargo test`，以免隐式执行未经安排的集成测试或文档测试。
骨架没有行为测试，零测试通过仅证明测试入口可用。新增逻辑后覆盖输入、输出、状态变化和必要失败路径。

有 actionlint 时还可检查 workflow：

```sh
actionlint .github/workflows/ci.yml
git diff --check
```

新文件在首次暂存前不会出现在普通 `git diff` 中；首次提交前还应检查暂存差异。

## Git 与配置文件

- 默认分支为 `main`；本地设置 `core.autocrlf=false`、`core.eol=lf`、`fetch.prune=true`。
- `.gitattributes` 固定仓库换行行为，`.editorconfig` 统一编辑器设置。克隆仓库后主要依赖这些受版本控制的规则。
- 不修改全局 Git 身份或用户全局配置；本地设置不会随克隆传播。
- 保留 `Cargo.lock`。忽略 `target/`、rumdl 缓存、编辑器输出、根目录运行配置、`.env` 和常见秘密文件。
- 所有实际部署配置集中在 `local/`；后续可提交 `examples/` 内无真实凭据的示例。任意命名的其他路径仍须人工核对。
- 不从模板推断许可证、GitHub owner、仓库公开性或镜像命名空间；这些在建立远程或发布前确定。

## GitHub CI 与后续发布

CI 在 `main` push、pull request 和手动触发时运行，使用 `ubuntu-24.04`，仅授予 `contents: read`。
同一 ref 新运行取消旧运行；每个 job 有超时；checkout 不持久化凭据。
Rust job 依次编译检查、格式检查、lint、单元测试；Markdown job 独立检查文档。

建立 GitHub 远程后，先运行一次 CI，核对 `Rust`、`Markdown` 两个 job 的实际结果，再将它们配置为合并必需检查。
分支保护是远程仓库设置，不由本地 workflow 自动启用；当前没有上传、提交、远程 URL 或托管 runner 的执行证据。

Dockerfile、容器运行检查及镜像发布在路线图部署阶段接入。届时验证非 root 运行、CA 证书、配置目录只读挂载、
容器内监听与宿主端口绑定、SIGTERM 清理、终止宽限时间及镜像架构。先有可运行服务，再声明可部署产物。
