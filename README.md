# Limen

Limen 是面向个人与内网使用的 Rust Chat Completions 网关：显式配置 provider 和模型别名，
通过本地 JavaScript ESM 配置有序请求与响应变换，主要部署目标为 Linux Docker。

`limen check` 加载并校验配置；`limen serve` 启动 HTTP/1 JSON 网关，提供
`GET /v1/models` 和 `POST /v1/chat/completions`。两个接口共用客户端认证，模型只来自显式配置。
全局 `onRequest` 后仅路由一次，非流请求依次执行 provider → model 的请求与响应 hook。
当前拒绝客户端流请求、hook 选择的流请求和上游 SSE；流协议与转换仍是后续工作。

## 检查与运行

安装 Rustup 和 C 编译工具链后，在仓库根目录执行：

```sh
cargo run --locked -- check --config examples/basic/server.yaml
```

默认配置路径为当前目录的 `server.yaml`；`config.js` 始终相对于该 YAML 所在目录定位。
CLI 使用 clap，支持 `--config PATH` 和 `--config=PATH`；`limen --help` 查看命令，
`limen check --help` / `limen serve --help` 查看对应选项，`limen --version` 查看版本。
示例使用保留域名和占位令牌，演示 `../shared/rules.mjs` 导入，不会连接上游。
实际配置放在被忽略的 `local/`，不要提交真实令牌。

将整个 `examples/` 目录复制到 `local/`，保留 `basic/` 与 `shared/` 的相对关系，
然后在 `local/basic/config.js` 中填写完整上游 endpoint、上游认证和模型映射，
在 `local/basic/server.yaml` 中设置客户端令牌。先检查，再启动：

```sh
cargo run --locked -- check --config local/basic/server.yaml
cargo run --locked -- serve --config local/basic/server.yaml
```

保留默认监听地址时，使用与 YAML 一致的客户端令牌请求：

```sh
curl http://127.0.0.1:8080/v1/models \
  -H 'Authorization: Bearer example-limen-client-token'
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}]}'
```

上述 curl 为 shell 示例；令牌是占位值，应替换为自己的本地配置值。
上游认证与客户端认证相互独立；客户端 Authorization、Cookie 和无关头不会默认转发。
一次生成至多发送一次，不自动重试或跟随重定向。响应默认使用对外别名，响应 hook 可显式改写。

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

退出码：`0` 检查成功、服务正常关闭、帮助或版本输出；`1` 配置、服务、运行时或关闭失败；`2` CLI 参数错误。
Ctrl-C 会停止监听并取消在途请求；Unix 下也处理 SIGTERM。VM 退役会关闭服务并以失败退出。

## 开发与边界

- [完整设计](docs/design.md)：配置 schema、管线、运行时与资源契约。
- [实施路线图](docs/roadmap.md)：当前状态、剩余任务和验收标准。
- [开发指引](docs/development.md)：构建、Rusteward、单元测试与隔离运行时实验。
- [工作区约定](AGENTS.md)：代理在本仓库工作时遵循的项目约束。

应用是 workspace 根包，源码位于 `src/`，配置和 JS 运行时库分别位于 `modules/config/` 与 `modules/script/`。
整体模块拆分建议见[设计中的工程边界](docs/design.md#12-workspace-与模块边界)。依赖由 workspace 根集中管理，
`Cargo.lock` 纳入版本控制。纯逻辑单元测试与有外部超时保护的运行时实验分开执行。

Linux 构建、Docker、真实上游及 GitHub runner 验证尚未完成，本机验证不能替代这些证据。
Windows 本机已有纯逻辑测试、隔离 JS 实验及本地合成 HTTP 上游实验，覆盖顺序、认证、取消、预算与关闭。
仓库尚未配置远程，项目尚未决定分发许可证，Cargo 包设置为不发布。
