# 记录与管理界面

`serve` 将生成摘要和 usage 保存到本机 SQLite，将运行日志写入滚动 JSONL 文件。配置、数据库与日志分别位于 config、data、logs；运行参数见[配置与接入](configuration.md)，挂载与权限见[部署与运维](deployment.md)。`check` 和离线 `test` 不创建运行数据库或日志。

## 认证与端点

在 `[auth]` 中设置独立的 `manager_key` 后，管理接口在同一监听地址的 `/admin/v1/` 下启用；省略该字段则返回 404。请求使用 `Authorization: Bearer <manager_key>`。推理用 `api_key`，两者不能互换，也不从 query 或 Cookie 取凭据。配置写入、在线重启和热重载没有接口。

## 浏览器查看

打开服务根地址，首页是一篇静态 LLM 科普文章。点击页脚的 `※`，输入 managerKey 后可查看用量、请求记录、运行日志、系统状态和配置。所有数据操作均使用现有只读管理接口；直接打开后台 hash 地址也需要验证凭据。

浏览器在当前站点的 localStorage 中保存 managerKey，刷新后重新验证。退出会清除保存的 key 和已加载的数据；凭据失效时也会退出。浏览器禁止或无法清理存储时，页面会显示具体提示。该存储不与其他站点共享。

用量和请求查询支持时间窗口、模型与 Provider 筛选，帮助标记说明时间归属、质量和缺失值含义。日志按文件及窗口翻页；窗口为空且仍有下一页时可以继续。服务重启或文件淘汰造成游标失效后，使用页面提供的重新读取操作，不会自动切换到其他历史文件。

已应用的筛选条件保存在 hash 地址中，刷新或前进后退可恢复；分页游标只保留在当前页面，刷新或改变筛选后从第一页开始。重复、未知或非法的筛选参数会显示错误，并暂停对应查询，可清除或重新应用筛选后继续。

界面和静态资源随二进制发布，不需要额外 Web 服务。页面路由使用 `/#/...`，不会占用推理或管理 API 的路径。

## API 契约

| GET 端点                         | 返回内容                                   |
| -------------------------------- | ------------------------------------------ |
| `/admin/v1/status`               | 进程、VM、存储和两个日志输出的健康状态     |
| `/admin/v1/config`               | 非秘密服务参数，或模型/provider 白名单投影 |
| `/admin/v1/requests`             | 持久化生成记录列表                         |
| `/admin/v1/requests/{requestId}` | 单条记录，不受默认时间窗口限制             |
| `/admin/v1/usage`                | 已知用量的有界汇总                         |
| `/admin/v1/logs/files`           | 活动日志及保留备份的身份与大小             |
| `/admin/v1/logs`                 | 一个日志文件内从新到旧的窗口               |

成功响应为 `{data: ...}`；列表另有 `nextCursor`，没有下一页时为 null。时间用 UTC RFC3339 毫秒格式，token 和 byte 计数用十进制字符串或 null。响应带 `Cache-Control: no-store` 与 `X-Limen-Request-Id`，不提供跨域许可。status 返回 200 只表示状态可读取；是否接收生成看 `acceptingGenerations`。

管理默认并发 4、总期限 2 秒，query 最多 4096 字节，响应最多 1 MiB。未知或重复参数、非法值及 GET 非空正文返回 400 `invalid_query`；无效认证为 401，未知端点/记录为 404，错误方法为 405。游标失效为 410 `cursor_expired`，范围过宽为 422 `query_too_broad`，繁忙/不可用为 503 `busy`/`storage_unavailable`，超时为 504 `query_timeout`。错误正文沿用 `{error: {code, message, request_id}}`，不输出原始错误信息。

## 请求与 usage

请求列表接受 `from`、`to`、`model`、`provider`、`state`、`outcome`、`limit`、`cursor`。from/to 同时提供，使用 RFC3339 的 `[from, to)` 窗口，按请求开始时间筛选，跨度最多 31 天；均省略时默认最近 24 小时。model/provider 为精确匹配，state 为 pending/finalized/interrupted，outcome 为 success/failed/cancelled。

列表默认 50 条、最多 100，按开始时间与记录序号倒序。后页可只传 cursor 和 limit，重复提供的过滤项必须与第一页一致。游标绑定当前运行，重启即失效；分页期间终态更新和保留清理仍可能改变记录，不承诺跨页快照。

usage 接受 from/to、model/provider 和 `groupBy=none|day|model|provider`，默认按 UTC 日汇总。每次最多扫描 100000 条、产生 128 组，超过后缩小查询窗口。响应 data 含 from/to、groupBy、retention 和 groups。每组包含记录数、状态/结果/质量/上游完成分类与 tokens；例如 `input: {sum: "123", samples: 2}`。无样本时是 `{sum: null, samples: 0}`，不是零。失败或取消请求的已知消耗仍参与统计，未知路由保留 null 分组。

初始记录确认提交后才执行生成管线；数据库不能接收记录时拒绝新生成。终态提交失败不改写已经发送的网络结果，可能留下 pending。重启将遗留 pending 标记为 interrupted，不推断成功、结束时间或消耗。记录为 finalized 表示网关管线已终结，不证明客户端收到全部字节。

默认保留 90 天、最多 100000 条；pending 不参与常规清理。清理后的历史不另存累计汇总，retention 中最早保留时间也不表示更早历史完整。数据库故障锁定后需修复并重启；不自动重试、切换备用存储或重放日志。

## 配置与日志检视

config 的 `section` 为 server（默认）、models 或 providers。server 只提供非秘密运行参数与 key 是否配置；列表默认 50 条、最多 100，可用 limit/cursor 分页。没有原始配置视图，不返回密钥、路径、脚本、环境变量、headers 或 defaults。

logs/files 返回 fileId、active、sizeBytes、firstTimestamp；fileId 来自首条完整记录的 log ID，文件名轮换不改变其身份。空文件不列出，损坏首记录只报告不可读状态。

logs 接受 fileId、cursor、limit、minLevel、requestId。默认读取活动文件，默认 100 条、最多 200 条；minLevel 为 trace/debug/info/warn/error，包含该级及更严重的记录。每次扫描最多 1 MiB，返回 `skippedRecords` 和 `scannedBytes`；半行与损坏行被跳过，不透出原始文本。空 data 配非空 nextCursor 表示当前扫描窗口未匹配，可以继续翻页。

后页可只传 cursor/limit，重复的 fileId 或过滤项必须一致。文件淘汰、截断、身份变化或服务重启返回 410，重新获取文件清单即可。接口不接受路径，不跨文件自动搜索，不提供下载、follow 或日志流订阅。

文件日志默认每份 16 MiB、保留 8 个备份，启动时轮换非空活动文件；另有 stderr 输出。每条日志最多 16 KiB，两路各有 1024 条有界队列。队列满会丢弃并计数；文件写入首次故障后停止该路输出，生成继续。status 分别报告队列/写入丢弃与 I/O 错误，日志不作为 usage 数据源。
