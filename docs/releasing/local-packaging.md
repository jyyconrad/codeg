# 本地打包（macOS DMG）

| 字段 | 值 |
| --- | --- |
| 文档标题 | 本地打包（macOS DMG） |
| 日期 | 2026-09-12 |
| 状态 | 本地安装测试用；不是 CI 发版流程 |
| 受众 | 在本机打桌面包的开发者 |
| 权威产物 | 只认 `src-tauri/target/release/bundle/` |

发版签名、公证、GitHub Release 见 [macos-signing.md](./macos-signing.md) 与 `.github/workflows/release.yml`。

---

## 1. 只打这一处

本机 ARM Mac 的桌面安装包 **只** 写到：

```text
src-tauri/target/release/bundle/dmg/codeg_<version>_aarch64.dmg
```

当前版本号与 `src-tauri/tauri.conf.json` 的 `version` 相同，例如 `codeg_0.30.6_aarch64.dmg`。同一次构建还会留下：

```text
src-tauri/target/release/bundle/macos/codeg.app
```

不要再产出或拷贝到下面这些位置：

| 路径 | 原因 |
| --- | --- |
| `src-tauri/target/aarch64-apple-darwin/release/bundle/` | 给 `pnpm tauri build` 加了 `--target aarch64-apple-darwin` 才会走这里。本机已经是 arm64，加 `--target` 只是换目录 |
| `~/Desktop/*.dmg` | 手工复制，不是构建产物 |
| 仓库根 `dist/` | Next 用词，与桌面包无关 |

Intel Mac 本机构建同样落在 `src-tauri/target/release/bundle/dmg/`，文件名是 `codeg_<version>_x64.dmg`。

---

## 2. 命令

在仓库根目录：

```bash
pnpm tauri:dmg
```

该入口会自动优先使用 rustup 的 `cargo` / `rustc`，并遵循仓库根的 `rust-toolchain.toml`。直接运行 Cargo 时也应让 rustup 代理优先：

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

等价于（不要加 `--target`）：

```bash
pnpm tauri build --bundles dmg --ci --config src-tauri/tauri.local-dmg.conf.json
```

`tauri.local-dmg.conf.json` 只关 updater 签名产物，避免本机没有 `TAURI_SIGNING_PRIVATE_KEY` 时构建失败。前端 `pnpm build` 和 sidecar `codeg-mcp` 仍由 `tauri:before-build` 自动跑。

sidecar 本机构建复用 `src-tauri/target/release/`，即使 Tauri 传入本机 triple 也不会再创建一套 `<triple>/release`。交叉编译保留目标目录；用于 Tauri 打包的 sidecar 文件名始终包含目标 triple。

开发缓存用 `pnpm rust:cache` 查看。dev/test 已关闭增量缓存；debug 超过默认 10 GiB 时，在停止开发服务、编译和编辑器自动检查后运行 `pnpm rust:cache:prune`。加 `--dry-run` 可预演，加 `--limit-gib N` 可调整阈值。清理只处理 debug，保留本节约定的 release 安装包；不要用无参数 `cargo clean` 代替。

完整的环境、验证命令和主动清理方式见 [本地构建与缓存维护](../building.md)。

可选：本机钥匙串里已有 Developer ID 时带上身份，Gatekeeper 少拦一层。没有也可以打，安装时用下文的「未公证」。

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID1234)"
pnpm tauri:dmg
```

本机身份用下面这条确认：

```bash
security find-identity -v -p codesigning | grep "Developer ID Application"
```

本地安装测试 **不要** 配 `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID`，否则 Tauri 会走公证。公证属于发版，见 [macos-signing.md](./macos-signing.md)。

---

## 3. 安装

```bash
open src-tauri/target/release/bundle/dmg/codeg_*.dmg
```

把 `codeg.app` 拖进「应用程序」，或：

```bash
cp -R src-tauri/target/release/bundle/macos/codeg.app /Applications/
```

未公证时若系统拦截：Control-点击图标 → 打开，或：

```bash
xattr -cr /Applications/codeg.app
open /Applications/codeg.app
```

---

## 4. 和 CI 发版的差别

| | 本地 `pnpm tauri:dmg` | CI tag 发版 |
| --- | --- | --- |
| 入口 | 本文件 | `.github/workflows/release.yml` |
| `--target` | **不加** | 矩阵里写死 triple |
| 产物目录 | `src-tauri/target/release/bundle/` | `src-tauri/target/<triple>/release/bundle/` |
| updater `.sig` | 关闭 | `createUpdaterArtifacts: true` |
| Apple 公证 | 不跑 | 缺 secret 则 macOS job 失败 |

CI 必须带 `--target`，因为同一套 workflow 要打 x64 / arm64。本地测试不要模仿那条命令。
