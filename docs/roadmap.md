# 实施路线图

## 当前状态与使用方式

`limen check` 与 `limen serve` 已接通配置、本地 ESM、有界 JS 调度、独立客户端认证和单次路由。
JSON→JSON、JSON→SSE、SSE→JSON、SSE→SSE 四路径及 provider → model 正序 hook 已实现。
配置、脚本和 Chat 协议库分别位于 `modules/config/`、`modules/script/` 与 `modules/chat/`；根包协调 HTTP 和生命周期。

Dockerfile、Compose、精简非 root 运行层、cargo-chef 依赖缓存、Rust 与 Buildx GHA 缓存已配置。
CI 在 main push 检查通过后构建 Linux amd64 镜像并推送 GHCR latest，成功后只保留该版本；
PR 和手动 CI 只构建，不推送。Linux CI 与容器镜像构建已跑通。

本文件是唯一进度文档，只保存当前状态、剩余任务、阻碍与待验证项；长期契约见[设计基线](design.md)，
命令、依赖选择和构建配置见[开发指引](development.md)。

## 待验证与后续工作

- 单独运行 `context-stream` JS 实验与 `transport-contracts` HTTP 实验，补齐流 hook、四种传输组合、
  usage 尾块、截断、取消及终态观察的运行证据；纯逻辑测试不能替代这些实验。
- 在 Linux 上验证 TLS/CA、SIGTERM、慢客户端、流背压及关闭传播。
- 验证容器启动、只读配置挂载、非 root 文件权限、端口绑定和关闭行为。
- 依据已跑通的 CI 配置分支保护。
- 分发前确定许可证；release 和真实上游验收单独安排。

## 首版之外

自动重试、故障转移、热重载、上游模型发现、UI、Responses/Embeddings/Images/音频接口、
多租户、配额、宿主网络/文件 API、多 VM 池，以及完整多模态跨形态转换均不在当前实施范围。
JSON Pointer helper 仅在真实重复逻辑出现后评估，不阻挡主链路实现。
