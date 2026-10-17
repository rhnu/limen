# 开发指引

## 准备与构建

在仓库根目录运行命令。Rust 工具链由 [`rust-toolchain.toml`](../rust-toolchain.toml)选择；QuickJS 和 TLS 依赖会编译 C 代码，本机需要 C/C++ 构建工具与 CMake。Windows MSVC 目标使用 Visual Studio C++ Build Tools 和 Windows SDK；Linux 构建基线见 [`Dockerfile`](../Dockerfile)。

安装 Node.js 24、根 `package.json` 指定版本的 pnpm、Rusteward 提供的 `cargo dev` 和 Markdown 检查器 `rumdl`。工程规则分别由 [`rusteward.toml`](../rusteward.toml)、[`Cargo.toml`](../Cargo.toml) 与 [`.rumdl.toml`](../.rumdl.toml)维护，工具版本与 CI 入口见 [工作流](../.github/workflows/ci.yml)。前端产物在 debug 和 release 构建中均直接嵌入二进制；首次 Rust 编译前先构建前端，运行时不需要 Node.js 或外部静态文件目录。

```sh
pnpm install --frozen-lockfile
pnpm build:web
cargo build --locked --bin limen
cargo run --locked -- --help
```

本机真实配置放 `local/config/`，运行数据与日志放 `local/data/`、`local/logs/`；容器挂载分别使用根目录 `config/`、`data/`、`logs/`；可运行的公共素材放 `examples/`，回归素材放 `fixtures/`。不要把本机凭据复制到示例或测试中。

当前处于[持久化与管理能力重构](design.md#持久化与管理能力重构)阶段，浏览器界面消费现有只读管理 API，环境验收按路线图收尾。期间不部署且无需保持阶段间兼容，不为旧配置、目录或实验 schema 添加过渡层。遇到 schema 不匹配应显式重建开发数据，不在启动失败路径自动删库。

## Rust 检查

按编译、格式、lint、行为验证的顺序执行：

```sh
cargo check --workspace --all-targets --locked
cargo dev format --check --diff --locked
cargo dev lint --locked --deny-warnings
cargo test --workspace --lib --bins --locked
```

需要修改格式时运行 `cargo dev format --locked`。`cargo dev check` 组合格式与 lint，不能替代编译检查或测试。局部修改可以先用包名、测试名缩小行为验证范围，例如：

```sh
cargo test --locked --package limen-chat --lib
```

常规测试包括纯逻辑、有限 VM 调用和合成上游的离线管线测试，已覆盖 `examples/testing` 的四种传输路径。CI 在打包后另行通过 CLI 运行 `examples/libraries`，验证打包产物及命令入口。新增测试验证输入、输出和失败后的状态，不断言源码文本或文件布局。

## 隔离测试

死循环、OOM、进程环境、真实 HTTP 和进程边界放在 `xtask` 的受监督子进程中。它们需要显式选择：

```sh
cargo xtask test --isolated --list
cargo xtask test --isolated
```

可在命令末尾传入 `--list` 返回的限定用例名；其他选项用 `cargo xtask test --help` 查询。监督进程负责外部超时、进程树终止和临时目录清理。不要将可能失控的 VM 用例直接放入常规测试进程。

隔离 HTTP 测试使用本地合成上游，并覆盖管理认证、持久化重启恢复和日志文件检视；`storage/*` 用例覆盖真实 SQLite 文件锁、恢复、保留容量、读写故障与取消。真实 provider 和 Docker 运行验收分别执行，不能用常规测试或离线用例替代。

## 前端与 JavaScript

pnpm workspace 管理根脚本工具、`web/` 和 `tooling/api-client/`，锁文件为 `pnpm-lock.yaml`。前端使用 React、Radix Themes、React Query、Zustand 和 Recharts；说明内容通过帮助标记按需展开。开发服务器仅代理管理 API 到本机 8080 端口：

```sh
pnpm dev:web
```

[`api/admin.openapi.json`](../api/admin.openapi.json)描述已有管理端点；修改后运行 `pnpm api:generate` 更新生成客户端。生成器单独使用 TypeScript 6，因为 Hey API 需要其 JavaScript 编译器 API；根脚本和前端仍使用 TypeScript 7。`api:check` 在临时目录重新生成并比较产物，只验证生成一致性，不代表后端响应行为通过。

```sh
pnpm api:check
pnpm build:examples
pnpm check:types
pnpm lint:web
pnpm format:web:check
pnpm test:web
pnpm build:web
```

需要格式化前端时运行 `pnpm format:web`；生成目录由生成器维护，不手工格式化。`test:web` 仅运行确定性的纯逻辑测试，覆盖数值、时间、认证与错误状态。浏览器布局、键盘操作和真实请求需另行检查，不以构建通过代替。

`build:examples` 生成第三方库示例的运行文件；修改入口是 [`examples/libraries/src/config.js`](../examples/libraries/src/config.js)。`check:types` 检查前端、类型声明、[`policies/`](../policies/) 与示例，包括预期会被类型系统拒绝的契约用法。策略行为的常规测试在真实 QuickJS 中导入库源码，离线组合用例复用网关管线，Node.js 静态检查不能替代它们。

静态检查不能证明 QuickJS 兼容性、JSON 合法性或协议语义。继续使用 [`limen check` 和离线用例](features/testing.md)验证实际模块；第三方库的打包边界见[脚本文档](features/scripting.md#第三方库)。

## Markdown 与交付

对本次修改的 Markdown 统一执行自动修复，再检查剩余诊断：

```sh
rumdl check --fix docs/design.md docs/development.md
rumdl check --deny-config-warnings
```

文件参数应替换为实际改动文件。内容变动时同时修复相对链接和锚点；架构结论更新到 `design.md`，工具入口维护在本文，剩余任务只留在 `roadmap.md`。

提交前检查暂存区，确认没有混入 `local/`、`config/`、`data/`、`logs/`、打包产物或任意位置的秘密。报告验证时区分编译与静态检查、常规测试、离线用例、隔离测试和部署验收；只报告实际执行的层次。

CI 在 Rust 编译前构建内嵌前端，并运行客户端生成一致性、JavaScript 类型、前端格式与 lint、纯逻辑测试、Rust 构建与测试、Markdown 和容器构建检查；main 推送仍发布 GHCR 镜像并清理旧版本。重构期间，[工作流](../.github/workflows/ci.yml)中的部署 Secrets 检查与 OliveTin webhook 调用已一起注释，即使 Secrets 配置齐全也不会触发部署。[触发脚本](../.github/scripts/trigger-deployment.sh)随注释入口保留，目前不在 CI 中执行；只有用户明确要求恢复部署时才重新启用。

此修改只停用仓库内 webhook。实施提交推送前还须停用仓库外跟踪 `latest` 的自动更新或固定 digest；不能把镜像发布成功当作部署被禁止的证据，也不自动恢复部署。外部环境操作的待办见[路线图](roadmap.md)。
