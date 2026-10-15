# Limen

[English](README.md) | [简体中文](README.zh-CN.md)

Limen 是使用 Rust 编写的 Chat Completions 网关，面向个人和内网使用场景。
通过显式模型别名连接上游 provider，并用本地 JavaScript 模块适配请求和响应。
主要部署目标为 Linux Docker。

## 功能

- OpenAI 风格的 `POST /v1/chat/completions` 与 `GET /v1/models` 接口。
- 显式模型别名、可复用的 provider 配置，以及独立的客户端和上游认证。
- provider → model 有序管线，可修改请求体、URL、请求头、响应和流事件。
- 同步与异步 JavaScript hook、本地 ESM 导入和请求独立状态。
- 通过 `limen.env(name)` 从进程环境变量读取上游凭据。
- 上下游 JSON/SSE 四种组合；跨形态转换支持文本、refusal、function tool calls、
  多 choice、finish reason 和 usage。
- 有界请求、队列、JavaScript 执行、事件展开和流聚合，明确报告超时与流截断。
- 配置检查、结构化元数据日志和 Docker 内的非 root 服务进程。

配置和 hook 示例见[功能与使用指南](docs/features.md)，后续工作见[路线图](docs/roadmap.md)。
当前专注于 Chat Completions；自动重试、故障转移、热重载、模型发现、UI 和其他 API 不在实现范围内。

## 从源码构建

需要准备：

- Rustup 和 [rust-toolchain.toml](rust-toolchain.toml) 固定的工具链。
- QuickJS 与 AWS-LC 所需的 C/C++ 编译工具链、CMake 和平台开发工具。
  Debian/Ubuntu 可安装 `build-essential`、`cmake` 和 `pkg-config`；Windows 使用 MSVC 构建工具与 CMake。

```sh
git clone https://github.com/rhnu/limen.git
cd limen
cargo build --release --locked --bin limen
```

生成的可执行文件为 `target/release/limen`，Windows 下为 `target/release/limen.exe`。

## 快速开始

以下命令采用 POSIX shell。PowerShell 可使用
`Copy-Item -LiteralPath examples -Destination local -Recurse` 复制配置目录。
curl 示例使用 POSIX 换行与引号语法，在 PowerShell 中需要调整，并调用 `curl.exe`。

1. 复制整个示例目录，保留共享模块的相对导入关系：

   ```sh
   cp -R examples local
   ```

2. 在 `local/basic/server.toml` 中设置客户端 bearer token，在 `local/basic/config.js` 中填写
   provider 的**完整 Chat Completions URL**、上游认证和上游模型名。
   示例中的 `.invalid` 地址和令牌均为占位值。真实配置保存在 `local/`。
   示例优先读取 `UPSTREAM_API_KEY`，设置方式见[环境变量指南](docs/features.md#从环境变量读取上游密钥)。

3. 检查配置，然后启动网关：

   ```sh
   ./target/release/limen check --config local/basic/server.toml
   ./target/release/limen serve --config local/basic/server.toml
   ```

默认监听 `127.0.0.1:8080`，`config.js` 始终从所选服务配置文件的同目录加载。
`check` 会执行顶层 JavaScript、导入模块和 top-level await，不连接 provider，也不运行请求阶段 hook。

### 调用 API

将下方令牌替换为服务配置中的客户端令牌。`chat` 是示例配置中的对外模型别名。

```sh
curl http://127.0.0.1:8080/v1/models \
  -H 'Authorization: Bearer example-limen-client-token'

curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}]}'
```

流式调用使用 `curl -N`，并在请求体中加入 `"stream":true`：

```sh
curl -N http://127.0.0.1:8080/v1/chat/completions \
  -H 'Authorization: Bearer example-limen-client-token' \
  -H 'Content-Type: application/json' \
  -d '{"model":"chat","messages":[{"role":"user","content":"Hello"}],"stream":true}'
```

支持设置 Chat Completions base URL 的客户端，可填写 `http://127.0.0.1:8080/v1`、
客户端 bearer token 和已配置的模型别名，并选择 Chat Completions 模式。

## Docker

需要 Docker Engine 或启用 Linux 容器的 Docker Desktop，以及 Docker Compose。
Compose 使用已发布的 Linux `amd64` 镜像 `ghcr.io/rhnu/limen:latest`。

首次部署前创建 `data/`，将独立的 Docker 示例复制进去：

```sh
mkdir -p data
cp -R examples/docker/. data/
```

PowerShell 先执行 `New-Item -ItemType Directory -Path data -Force`，再执行
`Copy-Item -Path examples/docker/* -Destination data -Recurse`。
在 `data/config.js` 中填写上游配置，在 `data/server.toml` 中设置客户端令牌。
这两个文件不会自动生成。准备好配置后执行：

```sh
docker compose pull
docker compose run --rm limen check
docker compose up -d
docker compose logs -f
```

使用 `docker compose down` 停止服务。Compose 将 `data/` 挂载到容器工作目录 `/data`，
检查与启动优先读取 `/data/server.toml`，仅在它不存在时读取 `/data/server.yaml`。仅向宿主的 `127.0.0.1:8080` 发布端口，
容器配置监听 `0.0.0.0:8080`。entrypoint 启动时以 root 将部署目录的组改为 `10001`，
设置组读取和目录访问权限，保留宿主文件属主，再以 `10001:10001` 执行 Limen。
部署目录需要可写挂载以便初始化权限，容器根文件系统保持只读。
真实配置与凭据不进入镜像构建上下文。
GHCR 拉取权限取决于包可见性和本机的 registry 登录凭据。
详细说明见[部署指南](docs/features.md#docker-部署)。

## 配置与运行

- `server.toml` 或 `server.yaml` 管理监听地址、客户端认证、资源限制和日志，两种格式使用相同字段、默认值和校验规则。
- 同目录的 `config.js` 导出 provider、模型别名、pipeline 和可选的全局 hook。
- 客户端认证与上游认证相互独立，默认不转发客户端 Authorization 和 Cookie。
- `onRequest` 后仅路由一次，每个阶段依次执行 provider 与 model 的步骤，数组内部保持正序。
- 原始客户端 `stream` 决定下游形态，hook 可独立选择上游形态；JSON → SSE 需要等待完整上游 JSON。
- SSE 成功以 `[DONE]` 结束；开始发送后失败会发送 `event: error` 并关闭连接，不补成功标记。
- 配置变更需要重启。Ctrl-C 停止监听并取消在途请求，Unix 下也支持 SIGTERM。

使用 `limen --help`、`limen check --help` 或 `limen serve --help` 查看命令选项，
`limen --version` 查看版本。不传 `--config` 时，两个命令优先读取当前目录中的 `server.toml`，
仅在它不存在时读取 `server.yaml`；其他 TOML 错误直接终止，不合并文件。
显式 `--config` 只读取指定文件，根据 `.toml`、`.yaml` 或 `.yml` 扩展名选择解析器。
错误、退出码、资源限制和 hook 契约见[使用指南](docs/features.md)。

## 文档与开发

- [功能与使用指南](docs/features.md)：配置、API、hook、流处理和故障排查。
- [设计](docs/design.md)：架构与契约，区分已确认决定和待验证事项。
- [开发指引](docs/development.md)：构建、Rusteward、单元测试与隔离运行时实验。
- [路线图](docs/roadmap.md)：剩余任务与后续工作。

参与开发前请阅读[工作区约定](AGENTS.md)和开发指引。修改中不得包含真实凭据，
验证结果应区分纯逻辑单元测试、运行时实验和部署验收。

## 许可证

Limen 使用 GNU Affero General Public License 第 3 版，限该版本（`AGPL-3.0-only`）。
完整条款见 [LICENSE](LICENSE)。
