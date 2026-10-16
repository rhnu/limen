# Limen 文档

Limen 是供个人与私网使用的 Chat Completions 网关。Rust 负责 HTTP、协议和资源边界，本地 JavaScript 负责模型配置与请求策略。

## 使用

| 目标                                      | 文档                                     |
| ----------------------------------------- | ---------------------------------------- |
| 配置监听、认证、provider 与模型别名       | [配置与接入](features/configuration.md)  |
| 编写 hook、使用策略库与观测、接入第三方库 | [JavaScript 脚本](features/scripting.md) |
| 理解 JSON/SSE 转换、流结束和错误          | [响应与流协议](features/transport.md)    |
| 用合成上游验证自己的配置与脚本            | [离线测试](features/testing.md)          |
| 部署 Docker、设置资源限制、查看日志和排错 | [部署与运维](features/deployment.md)     |

从仓库根目录检查随附示例：

```sh
cargo run --locked -- check --config examples/basic/server.toml
cargo run --locked -- test --config examples/testing/server.toml --cases examples/testing
```

示例上游地址使用保留域名；`check` 和 `test` 不连接上游。接入真实服务前，按[配置与接入](features/configuration.md)准备自己的配置。

## 参与开发

- [架构与约束](design.md)：组件职责、数据流及必须保持的边界。
- [开发指引](development.md)：构建、检查和测试入口。
- [路线图](roadmap.md)：尚未完成的仓库事项与验收。

配置完整类型见 [`types/limen.d.ts`](../types/limen.d.ts)，可运行素材见 [`examples/`](../examples/)。类型声明帮助编辑与静态检查，运行时校验仍以 Rust 实现为准。
