# JavaScript 脚本

## 模块与运行环境

`config.js` 必须是默认导出配置对象的 ESM。可用相对路径导入本地 `.js`、`.mjs`，包括 `../`；模块按规范化文件路径加载并受总源码大小和模块数量限制。不能直接导入包名、`node:`、URL、带 query/fragment 的路径或 import attributes。配置及其依赖在同一服务生命周期内保留，不随请求重新加载。

环境提供 ECMAScript 内建能力和 `limen.env()`，不提供 Node.js、DOM、文件系统、网络或计时器 API。`async`/Promise 可用，顶层 await 也受启动预算约束；没有宿主 I/O 能唤醒的悬空 Promise 只能等待超时。需要 npm 库时先[打包为本地 ESM](#第三方库)。

## Hook 顺序与可写内容

全局 `onRequest` 在路由之前执行。路由后，每个阶段都遍历 provider.pipeline，再遍历 model.pipeline，均按数组正序；step 名称在所属 pipeline 内必须唯一，每个 step 至少有一个 hook。是否进入响应或事件阶段取决于[四种传输路径](transport.md#四种传输路径)。

| Hook              | 触发点                            | 可修改内容                                       |
| ----------------- | --------------------------------- | ------------------------------------------------ |
| `onRequest`       | 认证、正文校验后，路由前          | `body`、`state`；可 `reject()`                   |
| `beforeSend`      | defaults 合并后，发送前           | `body`、`url`、`headers`、`state`；可 `reject()` |
| `afterReceive`    | 收到上游头后，读正文前            | `status`、响应 `headers`、`state`                |
| `onUpstreamEvent` | 每个真实上游 SSE chunk            | `event`、事件替换、`state`                       |
| `onResponse`      | 完整 JSON，包括 SSE 聚合结果      | `body`、`state`                                  |
| `onStreamEvent`   | 每个即将发给客户端的 chunk        | `event`、事件替换、`state`                       |
| `onError`         | 请求失败或取消后                  | 只读观察                                         |
| `onFinish`        | 每个终态，失败时位于 `onError` 后 | 只读观察                                         |

所有 hook 必须返回 `undefined` 或最终 resolve 为 `undefined` 的原生 Promise。通过修改 `ctx` 工作，不返回替代正文、事件、上下文或错误对象；`return ctx.body` 和 `return null` 都违反契约。

```js
const limitOutput = {
  name: "limit-output",
  beforeSend(ctx) {
    const requested = ctx.body.max_completion_tokens;
    ctx.body.max_completion_tokens = typeof requested === "number"
      ? Math.min(requested, 4096)
      : 4096;
    delete ctx.body.store;
  },
};
// 将 limitOutput 放入所需 provider 或 model 的 pipeline。
```

`body` 与 `event` 可直接修改或整体替换。`state` 引用固定，初始为 `{}`，其 JSON 字段可修改；不允许添加任意顶层上下文字段。每次调用成功后才提交通过校验的草稿，throw、reject 的 Promise、非法返回值或非法草稿都不会提交当前调用的修改。前序已提交的修改仍保留。

`beforeSend` 修改 `body.model` 或 `url` 只改变这次实际发送，不重新路由。不要用模块级可变变量保存请求数据：多个请求的 Promise 可交错完成，闭包副作用也不会因 hook 失败或取消回滚。

## 请求拒绝

在 `onRequest` 或 `beforeSend` 调用 `ctx.reject()`，默认返回 400；可指定 403 或 422。每次调用最多记录一次拒绝。它不会抛异常或提前结束函数，需要停止策略执行时自行 `return`：

```js
const tenantPolicy = {
  onRequest(ctx) {
    if (ctx.client.headers.getAll("x-tenant").length !== 1) {
      ctx.state.reason = "tenant-required";
      ctx.reject(403);
      return;
    }
  },
};
```

将 `tenantPolicy.onRequest` 设为配置对象的全局 `onRequest`，并配合下面的头部白名单。只有当前 hook 成功、草稿合法时拒绝才生效；此时有效 `state` 会提交，后续请求处理停止。throw、Promise 拒绝或破坏正文后再 `reject()` 会作为 hook 错误处理。客户端只收到固定的 `request_rejected` 错误，不会收到自定义正文。

## 头部与客户端上下文

服务配置需显式允许脚本读取业务头：

```toml
[script_context]
client_headers = ["x-tenant", "x-example-policy"]
```

`ctx.client.headers.getAll(name)` 不区分头名大小写，返回新的字符串数组；缺失或未暴露时返回 `[]`。不支持写入，也不会自动转发。认证、Cookie 和传输相关头不能加入白名单；客户端 `Connection` 指名的头也会被剔除。要传递业务值，在 `beforeSend` 中明确设置：

```js
const forwardTenant = {
  name: "forward-tenant",
  beforeSend(ctx) {
    const tenants = ctx.client.headers.getAll("x-tenant");
    if (tenants.length === 1) ctx.headers.set("X-Tenant", tenants[0]);
  },
};
```

将 `forwardTenant` 放入所需 provider 或 model 的 pipeline。`ctx.headers` 的 `getAll`、`append`、`set`、`delete` 操作也不区分大小写。`getAll` 返回副本，`append` 保留重复值，`set` 替换同名所有值。API 引用和方法固定，内容可以通过这些方法变更。

`Host`、`Content-Length`、`Content-Type`、`Content-Encoding` 及逐跳传输头由 Rust 管理，不能通过 headers draft 设置或删除。`afterReceive` 的初始响应头只保留上游 `Content-Language`、`X-Request-Id`、`Retry-After` 中可转发的值；它可以再添加业务响应头。

## 元数据、状态与观察

`ctx.meta` 始终只读：`requestId` 和 `phase` 标识当前调用，`model` 是选中的别名，`provider` 是配置名，`configuredUpstreamModel` 是配置中的 upstream。路由前后三者由 `null` 变为已选值；`clientStream` 保留原始请求的流意图。收到上游头后，`upstreamStatus` 和 `upstreamContentType`（`json`/`sse`/`other`）固定为原始事实，不随 `afterReceive` 修改。

`onError` 与 `onFinish` 获得只读的 `state`、`meta`、`client`、`stats` 和终态 `outcome`。失败时附带经过清理的 `error`，含 code、HTTP status、request_id、phase 和 step。stats 提供字节数、上游响应头耗时、总耗时、实际发送模型与去除路径和 query 的目标 origin；没有正文。

观察 hook 共享有限清理预算。观察失败只产生诊断，不改变请求终态；前一个观察耗尽预算或 VM 已退役时，后续观察可能不执行。不要依赖它们保证外部副作用完成。

## 流事件替换

事件 hook 默认继续发送修改后的 `ctx.event`；调用 `ctx.replaceEvents(events)` 可丢弃、替换或按顺序拆分事件，每次调用最多一次：

```js
const dropEmpty = {
  name: "drop-empty",
  onUpstreamEvent(ctx) {
    if (ctx.event.choices.length === 0 && ctx.event.usage == null) {
      ctx.replaceEvents([]);
    }
  },
};
```

将 `dropEmpty` 放入所需 provider 或 model 的 pipeline。替换数组中的每个事件都会依次进入后续 step。`onUpstreamEvent` 的输出用于聚合或继续流处理；`onStreamEvent` 的输出只用于下游 SSE。不能操作 `[DONE]`，它由 Rust 在整体成功后发送。

事件必须保持合法 Chat chunk 和 choice 完成语义。丢弃所有 choice、丢掉必要的 finish 事件或继续写入已完成 choice 会失败。一个源事件经过所有事件 hook 的累计产出受 `max_output_events`、`max_output_bytes` 限制，不能逐步展开绕过限额。

## JSON 边界

配置数据、state、正文与事件只能包含普通 JSON 值。函数仅能放在规定的 hook 槽位。桥接不依赖用户自定义 `toJSON`，拒绝循环、Proxy、getter/setter、Symbol、BigInt、`undefined` 字段、稀疏数组、额外数组属性和自定义原型对象。数值必须有限，整数必须在 JS 安全整数范围内。可选字段应省略，不要用 `undefined` 或 `null` 代替缺失的配置项。

JSON 深度、节点和保守字节预算覆盖整个 hook 输出，不只覆盖正文。JS 中能构造的对象不一定能提交到 Rust，静态类型检查也不能替代这些校验。

## 第三方库

选择只依赖 ECMAScript 的库，先在 Node.js 开发环境打包为不含外部导入的本地 ESM，再交给 QuickJS。构建工具不会为运行时提供 `fetch`、`process`、`Buffer`、文件系统或 DOM；目标语法可编译不代表库的宿主能力可用。

随附的 [Valibot 示例](../../examples/libraries/src/config.js)通过谓词校验请求，在 `onRequest` 拒绝无效参数，在 `beforeSend` 实施 provider 限制；保留原正文可避免校验器丢失未知扩展字段。构建脚本见 [`build.mjs`](../../examples/libraries/build.mjs)，生成文件为同目录的 `config.js`，不要直接编辑它。重分发库代码时保留其许可证。

安装、构建、静态检查命令统一见[开发指引](../development.md#javascript-静态检查)。打包后执行配置检查及[对应离线用例](testing.md#运行示例)，验证真实 QuickJS 行为。

## 脚本静态检查

[`types/limen.d.ts`](../../types/limen.d.ts) 是 JSDoc/TypeScript 的可选编辑契约，不是可导入的运行时模块。在默认导出的配置对象前添加 JSDoc：

```js
/** @satisfies {import("../../types/limen").Config} */
```

按文件位置调整类型路径，完整用法见 [`examples/basic/config.js`](../../examples/basic/config.js)。声明使用 readonly 区分上下文权限，检查 hook 返回值和已知字段；完整运行校验仍在 Rust。项目检查入口见[开发指引](../development.md#javascript-静态检查)，hook 实现入口见 [`src/gateway/`](../../src/gateway/) 与 [`modules/script/src/context.js`](../../modules/script/src/context.js)。
