# 开发指引

## 准备与构建

在仓库根目录运行命令。Rust 工具链由 [`rust-toolchain.toml`](../rust-toolchain.toml)选择；QuickJS 和 TLS 依赖会编译 C 代码，本机需要 C/C++ 构建工具与 CMake。Windows MSVC 目标使用 Visual Studio C++ Build Tools 和 Windows SDK；Linux 构建基线见 [`Dockerfile`](../Dockerfile)。

安装 Rusteward 提供的 `cargo dev` 和 Markdown 检查器 `rumdl`。工程规则分别由 [`rusteward.toml`](../rusteward.toml)、[`Cargo.toml`](../Cargo.toml) 与 [`.rumdl.toml`](../.rumdl.toml)维护，工具版本与 CI 入口见 [工作流](../.github/workflows/ci.yml)。

```sh
cargo build --locked --bin limen
cargo run --locked -- --help
```

本机真实配置放 `local/`，容器部署配置放 `data/`；可运行的公共素材放 `examples/`，回归素材放 `fixtures/`。不要把本机凭据复制到示例或测试中。

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

常规测试包括纯逻辑、有限 VM 调用和合成上游的离线管线测试。新增测试验证输入、输出和失败后的状态，不断言源码文本或文件布局。

## 隔离测试

死循环、OOM、进程环境、真实 HTTP 和进程边界放在 `xtask` 的受监督子进程中。它们需要显式选择：

```sh
cargo xtask test --isolated --list
cargo xtask test --isolated
```

可在命令末尾传入 `--list` 返回的限定用例名；其他选项用 `cargo xtask test --help` 查询。监督进程负责外部超时、进程树终止和临时目录清理。不要将可能失控的 VM 用例直接放入常规测试进程。

隔离 HTTP 测试使用本地合成上游。真实 provider 和 Docker 运行验收分别执行，不能用常规测试或离线用例替代。

## JavaScript 静态检查

Node.js 只用于开发工具，不是网关运行依赖。安装锁定的开发依赖并检查脚本：

```sh
npm ci --ignore-scripts
npm run build:examples
npm run check:types
```

`build:examples` 生成第三方库示例的运行文件；修改入口是 [`examples/libraries/src/config.js`](../examples/libraries/src/config.js)。`check:types` 检查类型声明、[`policies/`](../policies/) 与示例，包括预期会被类型系统拒绝的契约用法。策略行为的常规测试在真实 QuickJS 中导入库源码，离线组合用例复用网关管线，Node.js 静态检查不能替代它们。

静态检查不能证明 QuickJS 兼容性、JSON 合法性或协议语义。继续使用 [`limen check` 和离线用例](features/testing.md)验证实际模块；第三方库的打包边界见[脚本文档](features/scripting.md#第三方库)。

## Markdown 与交付

对本次修改的 Markdown 统一执行自动修复，再检查剩余诊断：

```sh
rumdl check --fix docs/design.md docs/development.md
rumdl check --deny-config-warnings
```

文件参数应替换为实际改动文件。内容变动时同时修复相对链接和锚点；架构结论更新到 `design.md`，工具入口维护在本文，剩余任务只留在 `roadmap.md`。

提交前检查暂存区，确认没有混入 `local/`、`data/`、打包产物或任意位置的秘密。报告验证时区分编译与静态检查、常规测试、离线用例、隔离测试和部署验收；只报告实际执行的层次。
