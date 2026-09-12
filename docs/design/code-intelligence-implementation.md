# Codeg 代码智能实现状态

| 字段 | 值 |
| --- | --- |
| 日期 | 2026-09-13 |
| 分支 | `develop-20260909` |
| 状态 | CodeGraph ACP 注入已完成；LSP MCP 门面待后续实现 |
| 对照方案 | `docs/superpowers/specs/2026-09-12-lsp-adapter-layer.md` |

## 已完成

- CodeGraph 在支持 `mcpServers` 的 ACP 会话中通过官方 `codegraph serve --mcp` 注入。
- 注入使用会话级配置，不修改 Claude/Cursor 等用户全局配置。
- 缺少二进制、索引或 Agent 不支持 MCP 时失败开放。
- CodeGraph 遥测、更新检查已关闭。
- Codeg native session 的 LSP pool 按 canonical workspace 复用；CodeGraph `init/sync` 按 workspace 去重。

## 当前边界

- 外部 ACP Agent 尚未获得 LSP MCP 工具。现有 `codeg-mcp` 是独立进程，不能直接访问 native session 的 `LspPool`。
- 现有 native LSP 工具仍是 Rig `lsp(operation=...)`，与目标的统一 MCP 门面并存。
- 设置变更会让新 session 使用新配置，旧 session 在释放前可能短暂保留旧 pool。

## 后续推进

1. 定义 `LspMcpRequest` 与 workspace pool lease，将 workspace 身份绑定到会话 token。
2. 扩展 `BrokerMessage`、UDS transport、companion schema 和 listener，转发 definition、references、hover、documentSymbol、workspaceSymbol、diagnostics。
3. 为每个请求接入取消链路，并复用现有 `LspPool`，避免按 Agent 或请求重复启动语言服务器。
4. 在 supports_mcp、总开关、服务器就绪和失败开放条件下补 ACP/Codeg/native 三类回归测试。

代码入口：`src-tauri/src/acp/connection.rs`、`src-tauri/src/agent/code_intel/`、`src-tauri/src/agent/tools/`、`src-tauri/src/bin/codeg_mcp.rs`。
