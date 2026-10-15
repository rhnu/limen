# 实施路线图

## 当前状态与使用方式

`limen check` 与 `limen serve` 已接通配置、本地 ESM、有界 JS 调度、独立认证和单次路由。
`limen test` 为每个用例创建独立 VM，以合成上游替代网络并复用生产管线；JavaScript 配置另有可选 JSDoc 类型检查。
脚本支持发送前的业务拒绝、显式白名单客户端头读取，以及经开发打包的纯 JavaScript 库示例。
JSON→JSON、JSON→SSE、SSE→JSON、SSE→SSE 四路径及 provider → model 正序 hook 已实现。
配置、脚本和 Chat 协议库分别位于 `modules/config/`、`modules/script/` 与 `modules/chat/`；根包协调 HTTP 和生命周期。
服务配置支持 TOML 与 YAML，共用 schema 和校验，默认 TOML 优先，仅缺失时回退 YAML。
常规 Cargo 测试包含纯逻辑、有限 VM 与离线管线组合；进程环境、失控执行、真实 HTTP 和进程边界
通过 `cargo xtask test --isolated` 单独监督。此前 VM/HTTP 验证已在 Windows 和 Linux CI 通过，
当前测试分层与命令见[开发指引](development.md)。

Dockerfile、Compose、精简运行层与入口降权、cargo-chef 依赖缓存、Rust 与 Buildx GHA 缓存已配置。
CI 在 main push 检查通过后构建 Linux amd64 镜像并推送 GHCR latest，成功后只保留该版本；
PR 和手动 CI 只构建，不推送。Linux CI 与容器镜像构建已跑通。

本文件是唯一进度文档，只保存当前状态、剩余任务、阻碍与待验证项；长期契约见[设计基线](design.md)，
命令、依赖选择和构建配置见[开发指引](development.md)。

## 待验证与后续工作

- 验证 Windows Job Object 在外部超时和父进程中断时的进程树回收与临时目录清理。
- 依据已跑通的 CI 配置分支保护。
- release 和真实上游验收单独安排。

## 首版之外

自动重试、故障转移、热重载、上游模型发现、UI、Responses/Embeddings/Images/音频接口、
多租户、配额、宿主网络/文件 API、多 VM 池，以及完整多模态跨形态转换均不在当前实施范围。
JSON Pointer helper 仅在真实重复逻辑出现后评估，不阻挡主链路实现。
