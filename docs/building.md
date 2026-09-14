# 本地构建与缓存维护

以下命令均在仓库根目录执行。本地 Rust 使用 `pnpm rust`，桌面使用 `pnpm tauri`，服务器使用 `pnpm server:dev` / `pnpm server:build`。这些入口统一编译器与产物目录。

## 环境

- 安装 Node.js 24，以及 [package.json](../package.json) 的 `packageManager` 指定版本的 pnpm。
- 通过 rustup 安装 Rust，遵循 [rust-toolchain.toml](../rust-toolchain.toml) 的 stable 通道。不要改用 Homebrew Rust，也不要给本仓库设置另一个 rustup override。
- 桌面构建仍需操作系统对应的 Tauri 编译依赖。

```bash
pnpm install --frozen-lockfile
pnpm rust --version
pnpm tauri --version
```

入口优先使用 `~/.cargo/bin`；配置了 `CARGO_HOME` 时使用其 `bin` 目录。Cargo 和 Tauri 的子进程都继承这一环境，避免 GUI 启动时误用 Homebrew 编译器。不要再临时覆盖 `RUSTC`、`RUSTUP_TOOLCHAIN`、`RUSTFLAGS` 或 `CARGO_INCREMENTAL` 来改变本地默认构建；确有需要时，应把用途和额外产物范围记录下来。

直接调用 Cargo 或使用编辑器自动检查时，也须让 rustup 代理在 PATH 最前。macOS/Linux 可在 shell 初始化配置的 PATH 设置末尾加入：

```bash
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

随后重新打开终端；已运行的编辑器需重启或重载 Rust 工具进程，才会继承新环境。CI Rust job 和 Docker 已准备 Rust，可继续使用其现有 Cargo 命令。stable 是统一发布通道，不代表不同时间的环境具有同一版本；升级工具链应集中进行。

## 开发与打包

| 场景 | 命令 | 说明 |
| --- | --- | --- |
| 前端开发 | `pnpm dev` | Next.js 开发服务 |
| 前端静态构建 | `pnpm build` | 输出到 `out/` |
| 桌面开发 | `pnpm tauri dev` | 自动准备前端与 MCP sidecar |
| 服务器开发 | `pnpm server:dev` | 运行 Rust 服务；浏览器界面需先 `pnpm build` |
| 服务器发布构建 | `pnpm server:build` | Rust 二进制输出到 `src-tauri/target/release/`；部署前端需另跑 `pnpm build` |
| 本机 macOS DMG | `pnpm tauri:dmg` | 详见 [本地打包](releasing/local-packaging.md) |

`pnpm rust` 会进入 `src-tauri` 再运行 Cargo，参数写法与 Cargo 一致。例如，vendor 独立测试使用 `pnpm rust test --manifest-path vendor/sacp-tokio/Cargo.toml --lib`。

## 验证

按修改范围执行相关检查；同一 target 内的 Rust 构建命令顺序运行，保持 feature 和编译参数稳定。

```bash
# 前端与构建脚本
pnpm eslint .
pnpm test
pnpm test:build-scripts
pnpm build

# 桌面 Rust
pnpm rust check --locked --features test-utils --all-targets
pnpm rust test --features test-utils
pnpm rust clippy --all-targets --features test-utils -- -D warnings

# 服务器与 MCP（不启用 Tauri）
pnpm rust check --locked --no-default-features --bins
pnpm rust test --no-default-features --bin codeg-server --lib
pnpm rust clippy --no-default-features --bin codeg-server --lib -- -D warnings
pnpm rust clippy --no-default-features --bin codeg-mcp -- -D warnings
```

## 产物目录

项目 [Cargo 配置](../.cargo/config.toml) 固定 target 根目录为 `src-tauri/target`，包括仓库内 vendor 的独立 Cargo 命令。本地 pnpm 入口也固定 Cargo 的 target/build-dir，不要另设临时 target 或把不同项目的 target 混放到全局目录。

| 内容 | 位置 |
| --- | --- |
| 本机 dev/test 产物 | `src-tauri/target/debug/` |
| 本机 release 产物 | `src-tauri/target/release/` |
| 本机 DMG | `src-tauri/target/release/bundle/dmg/` |
| 真正的交叉编译产物 | `src-tauri/target/<triple>/` |
| Tauri sidecar 暂存 | `src-tauri/binaries/codeg-mcp-<triple>`，Windows 带 `.exe` |
| 下载的 crate / Git 依赖 | `~/.cargo` 或配置的 `CARGO_HOME` |

本机 Tauri 构建不加 `--target`。sidecar 在目标等于 host 时复用本机 `release/`，只在交叉编译时加入 triple 目录；暂存文件名始终包含 triple。本地入口忽略继承的 `CARGO_BUILD_TARGET`，交叉编译必须显式传 `--target`。CI 的发布矩阵继续按现有流程显式指定目标。

## 缓存维护

dev/test 已关闭增量缓存，主项目保留回溯行号，依赖关闭调试信息。修改主库后编译可能变慢，未变化的依赖仍然复用。检查、测试、桌面和服务器需要不同产物，统一环境不会把它们压成一份，也不能保证容量永远不增长。

```bash
# 查看本机 debug 占用，默认预算 10 GiB
pnpm rust:cache

# 预演：仅超过预算才会列出清理目标
pnpm rust:cache:prune --dry-run

# 先停止开发服务、编译及编辑器自动检查，再清理超限 debug
pnpm rust:cache:prune

# 单次使用 8 GiB 预算；也可设置 CODEG_DEBUG_CACHE_LIMIT_GIB
pnpm rust:cache:prune --limit-gib 8

# 换 rustc / profile 后主动重置 debug，不受容量阈值限制
pnpm rust clean --profile dev
```

预算只覆盖本机 `target/debug`。超限时构建入口提示维护，不会自动删除；它不是单次构建峰值的硬上限。统计会去重硬链接，因此可能小于 Cargo clean 输出的逻辑字节总量。

`rust:cache:prune` 检测到 Rust 编译进程时会拒绝清理。进程检查只是快照，Cargo 整目录 clean 本身不持有构建锁，所以仍须先停止自动检查和构建。

以上清理保留 `release/bundle` 及其它目标目录。日常不必每次 clean；不要用无参数 `cargo clean` 代替，否则会删除整个 target，包括发布安装包。历史交叉编译缓存需单独确认范围再处理。
