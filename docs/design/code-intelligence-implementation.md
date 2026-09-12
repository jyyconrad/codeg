# Codeg 代码智能实现状态

| 字段 | 值 |
| --- | --- |
| 日期 | 2026-09-13 |
| 分支 | `develop-20260909` |
| 状态 | 已落地项目级 `ProjectCodeIntelSupervisor`：托管 LSP pool、CodeGraph `init/sync`/`serve --mcp` 与 Streamable HTTP MCP 适配器。Agent 会话只注入 MCP 连接（HTTP 优先，stdio-only 走传输连接器）。设置页改为 Tools 配置，展示 LSP/CodeGraph MCP 工具而非语言服务器。 |
| 对照方案 | `docs/superpowers/specs/2026-09-12-lsp-adapter-layer.md` |

## 已完成

- CodeGraph `init/sync` 按 workspace 去重，并由 Codeg 进程启动。
- ACP 会话注入项目级 Streamable HTTP MCP endpoint（`codeg-code-intel`）；stdio-only Agent 注入无业务逻辑的 `--code-intel-mcp-stdio` 连接器。
- 缺少二进制、索引或 Agent 不支持 MCP 时失败开放。
- CodeGraph 遥测、更新检查已关闭。
- Native session 通过 `ProjectCodeIntelSupervisor::acquire` 持有项目 lease，不再由会话私有 spawn 宿主 `init`/`sync`。

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
