# 部署与运维

## 本机与私网

先按[配置与接入](configuration.md)准备 `local/config/server.toml` 和同目录 `config.js`，执行 `check` 后启动 `serve`。发行构建可使用：

```sh
cargo build --release --locked --bin limen
```

默认监听 `127.0.0.1:8080`，可在服务配置中指定 IP 与端口。Limen 提供 HTTP/1 服务；需要公网 TLS、域名或访问边界时由外部反向代理处理。客户端与上游 token 分开管理。上游客户端不使用系统代理环境变量，也不跟随重定向或自动重试。

## Docker

仓库 [`compose.yaml`](../../compose.yaml)使用 GHCR 的 Linux amd64 镜像，将 `config/` 只读挂载到 `/config`，`data/`、`logs/` 分别读写挂载到 `/data`、`/logs`。首次部署时，在仓库根目录准备独立的 [Docker 示例](../../examples/docker/)配置。

POSIX shell：

```sh
mkdir -p config data logs
cp examples/docker/server.toml examples/docker/config.js config/
```

PowerShell：

```powershell
New-Item -ItemType Directory -Path config, data, logs -Force | Out-Null
Copy-Item -LiteralPath 'examples/docker/server.toml', 'examples/docker/config.js' -Destination config
```

在 `config/server.toml` 设置客户端 token，在 `config/config.js` 设置完整上游 Chat Completions URL、独立上游凭据和模型名；该示例可单文件加载。随后验证并启动：

```sh
docker compose run --rm limen check
docker compose up -d
docker compose logs -f limen
```

容器内服务应监听 `0.0.0.0:8080`，否则宿主映射端口无法访问。随附 Compose 将宿主端口绑定到回环地址；要开放私网访问，应按实际边界调整端口映射和客户端认证。

需要环境变量时，在本机 Compose 配置中为服务显式添加 `environment` 或 `env_file`。Compose 的 `.env` 插值不会自动把所有变量传入容器。完整挂载脚本及其相对导入依赖，容器不能读取宿主挂载范围以外的文件。

容器直接以 UID/GID 10001 运行，根文件系统只读，移除所有 capabilities，保留 `no-new-privileges`。没有 root 改权入口；部署者须预先保证配置及导入文件可读、目录可遍历，data/logs 可读写。在 Linux 首次准备目录时，可按实际宿主权限策略执行：

```sh
sudo chown -R 10001:10001 config data logs
sudo chmod -R u=rwX,go= config data logs
```

这些命令只应用于专门准备的 Limen 目录。Docker Desktop 的 bind mount 由宿主共享机制控制，请确认共享目录可写；`check` 仅验证配置，不证明运行挂载可写。

查看或检查指定配置可在命令后加 `--config /config/server.yaml`。配置变更后执行 `check` 并重启服务。自行构建镜像时使用：

```sh
docker build --platform linux/amd64 --target runtime -t limen:local .
```

构建依赖缓存、发布和镜像维护由 [`Dockerfile`](../../Dockerfile) 与 [CI](../../.github/workflows/ci.yml)维护；重构期间 CI 只构建、测试和发布镜像，webhook 部署触发已注释；恢复部署需另行明确授权。

## 资源限制

`[limits]` 可覆盖所需边界，未填项使用默认值。只调整实际遇到的限制；完整字段、默认值和取值范围以 [`ResourceLimits`](../../modules/config/src/server.rs)为准。

| 目标         | 相关设置与取舍                                                                                                                                  |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 请求期限     | `request_ms` 限制请求主流程，`hook_ms` 限制单次 hook 等待，取两者较早期限；终态入库使用 `storage.operation_ms`，错误适配与观察使用 `cleanup_ms` |
| VM 失控保护  | `execution_ms` 限制同步执行片段，`vm_work_ms` 限制连续微任务；`memory_bytes`、`stack_bytes` 限制 VM 资源                                        |
| 容量         | `concurrency` 限制在途请求，VM 使用 `queue_capacity`、`max_pending` 和 `job_batch`；容量满时拒绝，不能当作无限等待队列                          |
| 大正文与大流 | `max_body_bytes` 限制入站正文，`max_event_bytes` 限制单个事件；`max_output_events` 与 `max_output_bytes` 限制脚本事件展开                       |
| 聚合与加载   | `max_aggregate_bytes`、`max_aggregate_events` 限制 SSE 聚合；模块、源码和 JSON 树另有独立上限                                                   |
| 启停清理     | `startup_ms`、`cleanup_ms`、`shutdown_ms` 分别控制启动、终结观察与关闭                                                                          |

`max_output_bytes` 也用于 JSON 输出和 JS 桥接，不能仅按流事件估算；它不是整条 SSE 的总流量上限。调大事件限制仍可能被聚合、JSON 节点、VM 内存或进程内存限制拒绝。容器内存需要容纳 Rust、TLS、连接缓冲和 VM，不能只按 `memory_bytes` 配置。

## 日志与关闭

`[log]` 的 `level` 控制等级，`format` 控制 stderr 的 text/JSON 格式；日志文件固定为带 UTC 毫秒时间的 JSONL。默认单文件 16 MiB、8 个备份，每条最多 16 KiB，队列有界并在满时丢弃。文件写入或轮换故障后停止本次运行的文件输出，通过 stderr 与管理状态诊断，修复后重启。Docker stderr 使用 `local` 日志驱动，单文件 10 MiB、最多 3 个文件。

日志只记录批准的请求、阶段、模型、provider 等元数据，不包含正文、认证、Cookie、原始 query 或 JS 异常消息。usage 事实存入 SQLite，原生统计口径见[原生统计](scripting.md#原生用量与阶段耗时)。磁盘不可写或数据库故障会拒绝新生成，已有请求可以留下结果未知记录；不会补偿重放。备份应先停止服务，再复制整个 data 目录，配置另行备份；不要只复制运行中数据库主文件，也不要通过日志重建 usage。

排错先用 `X-Limen-Request-Id` 关联请求，必要时提高日志级别；提高级别也不会输出被省略的敏感内容。脚本故意抛出的说明文字不会出现在日志，应依靠命名清晰的 step、错误分类和安全位置定位。

Ctrl+C 或 Unix SIGTERM 会停止监听、取消请求，依次等待 HTTP 排空、存储关闭、VM 关闭，最后有限刷新日志。前三者分别使用 `shutdown_ms`，两路日志共 3 秒；默认总预算 18 秒，Compose 宽限 25 秒。修改 `shutdown_ms` 时，外部宽限需大于 `3 × shutdown_ms + 3000 ms` 并留调度余量。VM 退役会触发服务退出，需要由进程管理器重启。

## 排错

| 现象                            | 检查方向                                                                                                                  |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| `check` 失败                    | 配置扩展名、字段路径与行列、同目录 `config.js`、导入路径、环境变量；先区分 server_config、script_startup、business_config |
| 401                             | 客户端 token、单个 Bearer Authorization；与上游 token 分开核对                                                            |
| `unknown_model`                 | 客户端别名及 `onRequest` 是否改写 model；`GET /v1/models` 只列配置别名                                                    |
| `request_rejected`              | 指定 phase/step 的策略条件、客户端业务头白名单；这是明确策略拒绝                                                          |
| `hook_failed`                   | hook 返回值、可写权限、JSON 边界、事件合法性与输出限额；用离线用例复现                                                    |
| `busy` / 504                    | 在途数、VM 队列或 pending 容量、hook 等待和请求期限；同步/微任务失控还可能使 VM 退役                                      |
| `upstream_failed`               | endpoint、DNS/TLS、实际凭据和上游状态；网关不重试、不跟随跳转                                                             |
| `upstream_protocol` 或缺少 DONE | 上游原始媒体类型、完整 SSE 终止、choice 完成、跨形转换支持与聚合限额                                                      |
| Docker 无法访问或读取配置       | 容器监听地址、宿主端口映射、挂载依赖、UID/GID 10001 对 config 的读取和 data/logs 的读写权限                               |

复现脚本与转换问题优先使用[离线测试](testing.md)，不要为诊断打印真实正文或秘密。网络、容器或真实上游问题需在对应运行环境验证。
