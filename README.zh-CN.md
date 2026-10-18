# Limen

[English](README.md) | 简体中文

Limen 是供个人与私网使用的 Rust Chat Completions 网关。通过本地 JavaScript 适配 provider 和请求响应处理，以显式模型别名为客户端提供稳定名称。内置 OpenAI、DeepSeek 和 OpenRouter 工厂，提供有类型的专有 options；用法见[内置 provider](docs/features/configuration.md#内置-provider)与[离线示例](examples/providers/)。

## 快速开始

先获取本仓库并安装 Docker Compose，然后在仓库根目录的 POSIX shell 中运行以下命令。随附镜像面向 Linux amd64。

```sh
mkdir -p config data logs
cp examples/docker/server.toml examples/docker/gateway.js config/
```

启动前确保 UID/GID 10001 可读取 `config/`、可读写 `data/` 和 `logs/`；容器不会以 root 自动改权。

修改复制出的配置：

- 在 `config/server.toml` 中，将 `auth.api_key` 替换为客户端 token。
- 在 `config/gateway.js` 中，通过 `defineGateway()` 注册 provider 及其模型，设置完整上游 Chat Completions URL、独立的上游凭据和上游模型名。

验证并启动网关：

```sh
docker compose run --rm limen check
docker compose up -d
```

网关监听 `http://127.0.0.1:8080`。PowerShell 准备步骤、环境变量和运维说明见部署指南。

## 文档

详细指南使用简体中文：

- [文档导航](docs/README.md)
- [源码构建与开发](docs/development.md)
- [部署指南](docs/features/deployment.md)

采用 [AGPL-3.0-only](LICENSE) 许可。
