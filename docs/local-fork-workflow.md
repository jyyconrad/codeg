# 本地 fork 同步备忘

| 字段 | 值 |
| --- | --- |
| 文档标题 | 本地 fork 同步备忘 |
| 日期 | 2026-09-12 |
| 状态 | 本仓库相对源仓库的开发约定；不是上游贡献指南 |
| 受众 | 在本 fork 上做本地改造的开发者 |
| 源仓库 | https://github.com/xintaofei/codeg |

通用检查、架构和代码风格见根目录 [AGENTS.md](../AGENTS.md)。本地 DMG 见 [releasing/local-packaging.md](./releasing/local-packaging.md)。

---

## 1. 分支怎么放

```text
upstream/main     源仓库（只读）
      ↓  只允许快进
main              源仓库镜子，不在上面做功能
      ↓  只 merge
develop-20260909  集成分支，吃 main，再收各 feat
      ↓
feat/<短名>       一件事一条分支
```

- `main` 上不提交功能。只执行 `git merge --ff-only upstream/main`。
- 新改造从刚同步过的 `develop-20260909`（或 `main`）拉 `feat/` 分支，不要继续往集成分支上堆互不相关的大功能。
- 一条 topic 只做一件事：Agent、Toolbox、频道，不要混。

首次需要加源仓库 remote：

```bash
git remote add upstream https://github.com/xintaofei/codeg.git
```

---

## 2. 什么时候同步

| 时机 | 做什么 |
| --- | --- |
| 开始一天开发前 | `git fetch upstream`；`main` 有新提交就先合进当前分支 |
| 上游打 tag（`v0.30.x`） | 当天把 `main` 合进 `develop` |
| 准备改第 4 节的枢纽文件之前 | 先同步，再改 |
| 最多不超过一周 | 即使没发版也 merge 一次 |

固定命令：

```bash
git fetch upstream
git checkout main
git merge --ff-only upstream/main

git checkout develop-20260909   # 或当前 feat/*
git merge main
```

冲突当天解完并提交 merge。拖到下个上游版本，同一批枢纽文件会叠冲突。

合完至少跑：

```bash
pnpm test
pnpm rust check --features test-utils
```

碰了解析器、registry 或设置页时，再跑对应模块的 `pnpm rust test --features test-utils --lib`。

本地构建、验证命令都在仓库根执行，统一入口见 [构建与缓存维护](building.md)。Rust 编译器跟仓库根 [rust-toolchain.toml](../rust-toolchain.toml)：`stable`，与 CI、Docker 使用同一发布通道。不要为本仓库 `rustup override set homebrew`；Homebrew rustc 1.88 编不过当前 lockfile。

dev/test 已关闭增量缓存，产物统一放 `src-tauri/target`，依赖下载缓存在 `~/.cargo`。用 `pnpm rust:cache` 查看 debug 占用；超过默认 10 GiB 时，停止开发服务、编译与编辑器自动检查，再运行 `pnpm rust:cache:prune`（可加 `--dry-run` 预演）。换 rustc/profile 后要主动重置，可在空闲期执行 `pnpm rust clean --profile dev`。这些命令保留 release 安装包，日常不必每次清理。

---

## 3. 新代码放哪里

优先 **新文件、新目录**。枢纽文件只留胶水（注册、一个 match 臂、设置页入口）。

| 要做的事 | 放这里 | 不要放这里 |
| --- | --- | --- |
| Codeg Agent 工具 / 模式 / 压缩 | `src-tauri/src/agent/` | 在 `acp/connection.rs` 里写完整模型循环 |
| Codeg Agent 设置 UI | `src/components/settings/codeg-agent-*.tsx`、`src/lib/codeg-agent-*.ts` | 把大段字段塞进通用 ACP 设置页中部 |
| Toolbox | `src/components/toolbox/`、`src-tauri/src/commands/toolbox/` | 改消息列表布局来塞工具 |
| 类型 | 尽量独立模块；`types.ts` 只加联合成员或一行注册 | 在 `types.ts` 中部插入大段 interface |
| i18n | 独立命名空间，键加在该对象 **尾部**（如 `codegAgent.*`、`toolbox.*`） | 插到别人也在改的段落中间；漏改 10 个 locale 里的某几个 |

Agent 三层不要混：L1 宿主（registry / spawn / 权限卡）只接线；循环和工具留在 `src-tauri/src/agent/`。详见 [superpowers/specs/2026-09-11-codeg-agent-architecture.md](./superpowers/specs/2026-09-11-codeg-agent-architecture.md)。

碰枢纽文件时把提交拆开：实现（只动自己的目录）和胶水（只改 connection / registry 几行）分开。上游改大文件时，重放的是胶水，不是整段功能。

---

## 4. 枢纽文件（少改、先同步再改）

两边都会改这些文件。改之前先 merge `main`；不要顺手 rustfmt、重排 import 或改无关变量名。

| 文件 | 原因 |
| --- | --- |
| `src-tauri/src/acp/connection.rs` | ACP 生命周期，上游几乎每个版本都动 |
| `src-tauri/src/acp/registry.rs` | CLI 版本钉，发版必改 |
| `src-tauri/src/parsers/codex.rs`、`parsers/pi.rs` | 会话导入 / 历史卡片 |
| `src/lib/types.ts`、`src/lib/api.ts` | 所有智能体类型和命令面 |
| `src/components/message/message-list-view.tsx` | 消息列表和 overlay |
| `src/i18n/messages/*.json` | 10 个 locale 必须一起改 |
| `package.json`、`pnpm-lock.yaml` | 前端依赖；锁文件跟 `package.json` 一起改 |

相对当前 `main`：新目录可以很大；枢纽文件合计超过大约 200～300 行就停下来拆提交，并先同步。一条 topic 活过两个上游小版本还没合进 `develop`，先 merge `main`，不要继续加功能。

---

## 5. 不要做的事

- 不要在 `main` 上直接开发。
- 不要改 Codex / Pi 解析器，除非需求就是修导入或历史。
- 不要把 OpenCode / Pi / DeepSeek 的版本钉改成和上游不同，除非有意 fork 行为。
- 不要在功能提交里改无关 i18n 标点或排序。
- 冲突不要留到下一个上游 tag。
