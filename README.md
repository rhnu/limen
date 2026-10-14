# Limen

Limen 是面向个人与内网使用的 Rust Chat Completions 网关：显式配置 provider 和模型别名，
通过本地 JavaScript ESM 配置有序请求与响应变换，主要部署目标为 Linux Docker。

当前仓库提供工程骨架和实施计划，**尚未实现网关服务**。现有二进制仅说明未实现状态并以非零状态退出；
`serve`、`check`、HTTP 接口、JS hook 和 SSE 均待开发。构建通过不代表这些功能可用。

## 从这里开始

- [完整设计](docs/design.md)：产品范围、配置、管线、异步运行时、流转换与安全边界。
- [实施路线图](docs/roadmap.md)：当前工作状态、依赖顺序、任务和阶段验收标准。
- [开发指引](docs/development.md)：工具、目录、检查命令、Git 和 CI 约定。
- [工作区约定](AGENTS.md)：后续代理在本仓库工作时应遵循的项目约束。

## 检查骨架

安装 Rustup 后，仓库的 `rust-toolchain.toml` 会选择固定工具链并要求 rustfmt、Clippy。

```sh
cargo check --workspace --all-targets --locked
cargo build --workspace --locked
```

完整质量检查及 Rusteward、rumdl 安装方式见[开发指引](docs/development.md)。
当前没有业务依赖和行为单元测试；`Cargo.lock` 随应用提交。

## 仓库边界

目前只有本地 Git 仓库，尚未配置远程、创建提交或运行 GitHub Actions。
CI 文件在推送到 GitHub 后检查 Rust 与 Markdown；仓库规则、分支保护和发布策略需在远程建立后配置。

实际配置和令牌应放在被忽略的 `local/` 目录中，示例只使用占位值。
项目尚未决定分发许可证，Cargo 包设置为不发布；不要将现有骨架当作发布产物。
