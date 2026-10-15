# 离线测试

`limen test` 使用真实配置、QuickJS hook 和网关请求/响应管线，把上游 HTTP 替换为本地合成响应。它不监听端口、不访问网络，适合在接入真实 provider 之前验证策略与 JSON/SSE 行为。

## 运行示例

在仓库根目录运行：

```sh
cargo run --locked -- test --config examples/testing/server.toml --cases examples/testing
```

该目录演示全部四种 JSON/SSE 组合。`--cases` 可指向一个 `.case.json` 文件或目录；目录只扫描本层文件，按文件名排序。每个用例创建新的服务和 VM 生命周期，不继承前一用例的闭包状态。

第三方库示例需先按[开发指引](../development.md#javascript-静态检查)构建，再执行：

```sh
cargo run --locked -- check --config examples/libraries/server.toml
cargo run --locked -- test --config examples/libraries/server.toml --cases examples/libraries
```

命令输出每个用例的 PASS/FAIL 和合计；失败比较只输出分类名称，不打印实际正文、请求头或期待值。有任一失败或执行错误时退出码非零。

## 编写用例

用例与自己的 `server.toml`、`config.js` 可以放在 `local/` 下。下面的用例使用别名 `chat`，在 JSON → JSON 路径检查响应别名和成功状态：

```json
{
  "name": "alias-response",
  "request": {
    "model": "chat",
    "messages": [{ "role": "user", "content": "Hello" }]
  },
  "upstream": {
    "status": 200,
    "headers": { "content-type": ["application/json"] },
    "body": { "model": "upstream-model", "result": "hello" }
  },
  "expect": {
    "sent": true,
    "response": {
      "status": 200,
      "body": { "model": "chat", "result": "hello" }
    }
  }
}
```

fixture 中的非标准 JSON 响应用于检查网关保留对象字段的行为；要验证真实 Chat 转换，应使用完整 Chat response/chunk，参考随附四路径示例。

`request` 是客户端正文，`clientHeaders` 可添加业务头，值为字符串数组。runner 自动提供服务配置的客户端凭据和 JSON Content-Type，fixture 不能覆盖这两个头。客户端上下文仍按服务白名单暴露。

`upstream` 定义 status、headers 与 body；使用 SSE 原始字节时改用 `bodyFile`，路径相对用例文件。body 与 bodyFile 不能同时出现；均省略表示空正文。请求被策略拒绝时仍需提供 upstream 定义，但它不会被使用。

## 断言结果

每个用例至少声明一个有效输出断言，按需选择以下类别：

| 断言                                   | 用途                                        |
| -------------------------------------- | ------------------------------------------- |
| `sent`                                 | 是否发送了上游请求                          |
| `outbound.url` / `headers` / `body`    | 最终发送的 URL、请求头和正文                |
| `response.status` / `headers` / `body` | 客户端状态、头和完整 JSON                   |
| `response.events` / `done`             | 下游 SSE 的有序 JSON chunk 列表与成功终止符 |
| `state`                                | 最后一次成功提交的请求状态                  |
| `error`                                | 终态 code、status、phase                    |
| `observationFailed`                    | 观察 hook 是否失败或无法执行                |

已声明的对象、数组和头集合做完整相等比较，不是子集匹配。头名按 HTTP 规则处理，重复值使用数组。未声明的普通输出字段不比较；`error` 和 `observationFailed` 缺省分别断言无请求错误、无观察失败。

例如验证策略拒绝，应同时检查未发送和稳定错误分类：

```json
{
  "sent": false,
  "error": {
    "code": "request_rejected",
    "status": 403,
    "phase": "onRequest"
  }
}
```

这是 `expect` 对象的内容，需放入完整用例。截断流用例应声明错误、`done: false`，并可检查已输出事件；SSE 的 `event: error` 不计入 `response.events`，错误本身用 `expect.error` 断言。

失败分类可直接定位检查边界：`outbound.body` 指向请求策略，`response.events` 指向事件变换，`state` 指向提交，`error` 指向终态。fixture 使用合成数据；即使比较器隐藏值，也不要把真实 token、正文或 Cookie 保存进仓库。

## 验证边界

离线测试能验证模块加载、路由、hook 顺序与提交、客户端上下文、事件替换、协议转换和失败终态。它不能证明真实 TLS/DNS、网络分块与背压、监听和断连、信号处理、容器权限或真实 provider 行为。

常规 Rust 测试与受监督隔离测试的入口见[开发指引](../development.md)。用例 schema 的权威定义是 [`src/offline/schema.rs`](../../src/offline/schema.rs)，更多策略和失败示例见 [`fixtures/gateway/`](../../fixtures/gateway/)。
