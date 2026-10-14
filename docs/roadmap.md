# 实施路线图

## 当前状态与使用方式

`limen check` 与 `limen serve` 已接通配置、本地 ESM、有界 JS 调度、独立客户端认证和单次路由。
JSON→JSON、JSON→SSE、SSE→JSON、SSE→SSE 四路径及 provider → model 正序 hook 已实现。
配置、脚本和 Chat 协议库分别位于 `modules/config/`、`modules/script/` 与 `modules/chat/`；根包协调 HTTP 和生命周期。

Dockerfile、Compose、精简非 root 运行层、cargo-chef 依赖缓存、Rust 与 Buildx GHA 缓存已配置。
CI 仅构建 Linux amd64 镜像，不推送、不发布、不运行容器。按本轮要求，Linux 与容器运行验收单独安排。

本文件是唯一进度文档，只保存当前状态、剩余任务、阻碍与待验证项；长期契约见[设计基线](design.md)，
命令、依赖选择和构建配置见[开发指引](development.md)。

## 验证证据与剩余门槛

已有 Windows 本机纯逻辑测试、隔离 JS 实验及本地合成 HTTP 上游实验，覆盖此前的非流请求、认证、顺序、失败提交、
资源拒绝、观察失败、deadline、断开、Ctrl-C 与退役退出。本轮流协议、转换、预算与内存正文测试属于纯逻辑验证，
不等于新流路径已经通过 HTTP、JS 运行、Linux、Docker、真实上游或 GitHub runner 验收。

JSON 连接期限已接线；Windows 系统写入排队未形成可靠的阻塞写证据，Linux 慢客户端实验仍待补充。
JS 自行捕获的 OOM 无法通过当前安全 API 可靠观察；内存上限仍生效，传播到宿主的 OOM 会退役。
不能取消 JS continuation、不能回滚闭包副作用的边界保留在设计中。

### 流路径的独立运行验收

- [ ] 单独运行新增 `context-stream` JS 实验，确认替代输出、异步数组增长、取消与迟到结果。
- [ ] 单独运行更新后的本地 HTTP `transport-contracts` 实验，确认四路径、原始客户端意图、usage 尾块及流内截断错误。
- [ ] 以分块上游、慢客户端和断开实验验证实际背压、取消、并发许可释放与有限关闭。
- [ ] 在 Linux 补充慢写连接期限、SIGTERM 与流清理证据。

### Linux 容器与首版交付

- [ ] 在 Linux amd64 上运行 CI 编译与单元测试、镜像构建，确认 QuickJS/AWS-LC C 构建、allocator、动态库与 TLS/CA。
- [ ] 验证完整配置目录只读挂载、共享模块导入、check、serve、宿主回环端口与非 root 读取权限。
- [ ] 验证 SIGTERM、在途请求和 pending 清理、有限关闭预算；本地合成上游覆盖 JSON、SSE、认证、错误和断开。
- [ ] 真实 provider 验证单独安排并记录范围。
- [ ] 明确许可证、仓库可见性、镜像位置和发布权限后，再接入推送与 release。
- [ ] 核对首版契约与剩余风险，在取得目标平台证据后完成首版验收。

配置与秘密不得进入镜像层或导出的构建缓存；进程异常恢复依赖部署重启策略，不重放生成请求。
发布多架构、原生 Windows、性能指标均不自动包含在首版承诺中。

## 首版之外

自动重试、故障转移、热重载、上游模型发现、UI、Responses/Embeddings/Images/音频接口、
多租户、配额、宿主网络/文件 API、多 VM 池，以及完整多模态跨形态转换均不在当前实施范围。
JSON Pointer helper 仅在真实重复逻辑出现后评估，不阻挡主链路实现。
