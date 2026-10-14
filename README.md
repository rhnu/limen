# Limen

Limen 是面向个人与内网使用的 Rust Chat Completions 网关：显式配置 provider 和模型别名，
通过本地 JavaScript ESM 配置有序请求与响应变换，主要部署目标为 Linux Docker。

当前支持配置加载和 `limen check`：解析 `server.yaml`、求值本地 ESM、保存 hook 并验证模型与 provider。
**HTTP 网关尚未实现**；`serve` 共用配置加载流程，校验后以退出码 `3` 明确报告服务不可用。
请求管线、客户端认证接线、上游调用及 SSE 是后续工作。

## 使用 check

安装 Rustup 和 C 编译工具链后，在仓库根目录执行：

```sh
cargo run -p limen --locked -- check --config examples/basic/server.yaml
```

默认配置路径为当前目录的 `server.yaml`；`config.js` 始终相对于该 YAML 所在目录定位。
示例使用保留域名和占位令牌，演示 `../shared/rules.mjs` 导入，不会连接上游。
实际配置放在被忽略的 `local/`，不要提交真实令牌。

最小基础配置：

```yaml
client_auth:
  bearer: example-limen-client-token
```

默认监听地址为 `127.0.0.1:8080`；无认证必须显式配置 `client_auth: {none: true}`。
基础配置、provider、model 和步骤均拒绝未知字段。资源字段、允许范围及业务配置见[设计](docs/design.md)。

`check` **会执行配置和导入模块的顶层 JavaScript**，包括 top-level await。它不监听端口、不请求 provider，
也不执行请求阶段 hook。通过检查只代表配置结构和启动求值有效，不能证明 hook 在所有输入上正确。
脚本不提供 Node、网络、定时器、进程或任意文件读写 API；本地模块导入是允许的文件访问。

退出码：`0` 检查成功或帮助输出；`1` 配置、运行时或关闭失败；`2` CLI 参数错误；`3` 服务尚未实现。

## 开发与边界

- [完整设计](docs/design.md)：配置 schema、管线、运行时与资源契约。
- [实施路线图](docs/roadmap.md)：当前状态、剩余任务和验收标准。
- [开发指引](docs/development.md)：构建、Rusteward、单元测试与隔离运行时实验。
- [工作区约定](AGENTS.md)：代理在本仓库工作时遵循的项目约束。

应用位于 `apps/limen`，配置和 JS 运行时库位于 `modules/`。依赖由 workspace 根集中管理，
`Cargo.lock` 纳入版本控制。纯逻辑单元测试与有外部超时保护的运行时实验分开执行。

Linux 构建、Docker、真实上游及 GitHub runner 验证尚未完成，本机验证不能替代这些证据。
仓库尚未配置远程，项目尚未决定分发许可证，Cargo 包设置为不发布。
