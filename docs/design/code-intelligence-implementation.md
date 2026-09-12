# Codeg 代码智能实现状态

| 字段 | 值 |
| --- | --- |
| 日期 | 2026-09-13 |
| 分支 | `develop-20260909` |
| 状态 | CodeGraph 索引任务已接入；当前 ACP 注入仍为会话级 stdio，项目级 MCP 宿主和 LSP MCP 适配待后续实现 |
| 对照方案 | `docs/superpowers/specs/2026-09-12-lsp-adapter-layer.md` |

## 已完成

- CodeGraph `init/sync` 按 workspace 去重，并由 Codeg 进程启动。
- 当前仍把官方 `codegraph serve --mcp` 作为 ACP 会话级 stdio 配置注入；这只是临时兼容路径，不符合目标的项目级进程托管。
- 缺少二进制、索引或 Agent 不支持 MCP 时失败开放。
- CodeGraph 遥测、更新检查已关闭。
- Codeg native session 的 LSP pool 已按 canonical workspace 复用，但生命周期尚未提升为独立项目 supervisor。

## 当前边界

- 外部 ACP Agent 尚未获得 LSP MCP 工具。适配器应直接访问项目 supervisor/LSP pool，不扩展 `codeg-mcp` 的内部 Broker 协议。
- 现有 native LSP 工具仍是 Rig `lsp(operation=...)`，与目标的统一 MCP 门面并存。
- 设置变更会让新 session 使用新配置，旧 session 在释放前可能短暂保留旧 pool。

## 后续推进

1. 建立按 canonical workspace 管理的 `ProjectCodeIntelSupervisor`，统一托管 LSP、CodeGraph `init/sync` 和 `serve --mcp` 子进程。
2. 在适配器层提供最小 LSP MCP 工具集，直接路由到 supervisor/LSP pool；取消沿用 MCP 请求取消和本地 cancellation token，不新增 `BrokerMessage`、UDS 或 companion schema。
3. 优先通过 Streamable HTTP 将项目级 MCP endpoint 注入支持 HTTP 的 Agent；仅对 stdio-only Agent 评估无业务逻辑的传输连接器，底层引擎仍由项目进程托管。
4. 补项目切换、workspace 隔离、进程退出、MCP 能力协商和失败开放回归测试。

代码入口：`src-tauri/src/acp/connection.rs`、`src-tauri/src/agent/code_intel/`、`src-tauri/src/agent/tools/`、`src-tauri/src/bin/codeg_mcp.rs`。
