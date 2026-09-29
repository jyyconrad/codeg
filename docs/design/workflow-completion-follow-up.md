# Workflow 结束后自动回传模型方案

## 目标与范围

当当前会话启动的 workflow 进入 `completed` 或 `failed`，Codeg 读取该次运行的最终总结或异常说明，向同一模型会话发送一条由宿主生成的后续消息。模型据此核对用户原目标，自行决定继续必要工作、处理失败，或结束并向用户汇报。`stopped` 表示取消或中断，不自动续跑。

本方案针对 ACP workflow。ACP 的 `session/prompt` 接受 `PromptInputBlock`，没有可供 Codeg 插入的独立 system-role 消息；此处的“Message”是一次带明确宿主来源的文本 prompt，进入模型会话历史。内置 Codeg Agent 的 Rig `Message` 存储不在本次链路内。

## 已核验的现状

1. Grok `workflow_updated` 和 AIR `taskType=workflow` 已适配为统一的 `AcpEvent::Workflow`。事件既可能在前台回合内到达，也可能在回合结束后到达。`WorkflowRun` 按 `run_id` 合并，保留终态记录、`result_summary` 和 `last_event_detail`；Grok 还用 `revision` 拒绝旧帧。参见 `src-tauri/src/acp/connection.rs`、`acp/workflow_adapt.rs`、`acp/session_state.rs`。
2. 前端只在观察到非终态到终态的变化时发桌面通知，没有模型回传。Web attach 的快照/重放和桌面多视图使前端不适合作为自动发送的唯一责任方。参见 `src/contexts/acp-connections-context.tsx` 的 `workflow` 事件分支。
3. 后端生命周期订阅器目前过滤 `AcpEvent::Workflow`。现有 `TurnComplete` 只表示模型的一个 prompt 回合结束，与 workflow 终态不是同一事件。参见 `src-tauri/src/acp/lifecycle.rs`。
4. `ConnectionManager::send_prompt_linked_with_source` 已处理会话绑定、状态更新和发送锁；`send_prompt_inner` 在 `turn_in_flight` 时拒绝并发 prompt。定时器唤醒提供了宿主自动发送的参考，但当前 `PromptSource` 只有 `User` / `SessionTimer`，非委托来源还会广播普通 `UserMessage` 和 `UserPromptSent`。参见 `src-tauri/src/acp/manager.rs`。
5. Grok 完成事件的 `result_summary` 是已验证的完整结果来源，真实样本超过 16K 字符；失败事件可能只有 `last_event_detail`。`load_grok_workflow_conclusion` 能读 scratch `report.md`，但当前实时链路没有调用它，不应假设文件一定存在或比终态事件更新。AIR 的 `summary` 映射为 `result_summary`，没有独立 error 字段。参见既有 [终态结果展示方案](../superpowers/specs/2026-09-17-workflow-result-display-design.md)。
6. Claude 的后台 `<task-notification>` 后可能由代理自行继续工作，且这段活动未必有可靠的 ACP 回合结束事件。参见 `src-tauri/src/acp/background_watch.rs`。因此宿主回发要有按代理/事件来源的策略，不能把所有终态通知等同于“模型还不知道结果”。

## 方案选择

采用后端的 workflow 终态协调器：在统一事件合并之后登记完成通知，由同一连接的发送入口串行投递。前端继续展示状态和通知，只增加宿主消息的来源标识。

不采用前端 `useEffect`/`sendPrompt` 自动发送：浏览器关闭后不会运行，多视图可能重复发送，快照重放可能把历史终态误判为新完成。也不在 workflow 事件处理处直接调用 `session/prompt`：该事件可能到达于正在运行的模型回合，现有并发门会拒绝它。

### 交付策略

- 宿主唤醒使用白名单，列在 `host_wake_policy`。当前唤醒 Grok、Claude Code、Codex：Grok 以 `workflow_updated` 报告终态；Claude Code 与 Codex 是被宣告 `asyncTasks` 的两个适配器，`taskType=workflow` 会适配成同一条终态边沿。
- OpenCode 尚未被宣告 `asyncTasks`。Codeg Agent、自定义代理以及其他内置代理没有 workflow 通道，投递记为跳过，原因是 `host wake disabled`。新增 `AgentType` 必须在该函数里显式选择唤醒或跳过。
- Claude 的原生 `<task-notification>` 仍可能在终态后自行继续。白名单接受这次重叠：宿主仍向同一会话发送一条普通文本提示。Codex 已核实的 `asyncTasks` 样本是后台终端；普通 shell 不会被适配成 workflow，只有 `taskType=workflow` 的帧才会唤醒。
- `stopped` 不回发；终态事件缺少可用文本时仍可发送状态事实，但必须明确写“未提供总结/异常详情”，不得把当前阶段名猜作结论。

白名单上的代理在 `completed` / `failed` 时走同一条发送路径。代理策略须有测试和可观测的日志，不能按 UI 是否打开来判断。

## 事件与投递流程

1. `SessionState::apply_event(AcpEvent::Workflow)` 在接受增量后比较旧/新状态，只在本会话已知运行首次进入 `completed` / `failed` 时产生待投递条目；`stopped` 后被有效增量修正为这两种状态也应触发。条目保存 `(agent_type, external_session_id, conversation_id, run_id)`、投递状态和终态文本证据；发送前的有效终态修订可更新最终状态与证据。终态后等待 500 ms 合并窗口；若仍无总结/异常详情，最多等待 3 秒再发送明确的“未提供详情”状态消息。首次看到的历史终态快照、旧 revision 和重复终态帧不创建新条目。已发送后再出现状态修正只更新展示，不自动发送第二次。
2. 生命周期订阅器只将 `completed` / `failed` 的 `Workflow` 增量纳入工作队列，并唤醒该连接的投递器；进度帧继续在事件总线处过滤。订阅器不在事件总线热路径上做文件读取或模型调用。为避免订阅器漏帧留下待投递项，连接空闲检查也扫描待投递队列；存在待投递项时不能让 idle sweep 断开连接。
3. 投递器在同一连接的 `prompt_lock` 下检查会话 id、连接存活、策略和 `turn_in_flight`。若仍有模型回合，保留待投递项，由 `TurnComplete` 后再次冲刷；若已空闲，使用 `PromptSource::WorkflowCompletion` 走 `send_prompt_linked_with_source`。这个来源不能取消用户设的 session timer、改写首条用户标题，或触发 `UserPromptSent` 人类消息通知。
4. 为每个 `(conversation_id, external_session_id, run_id)` 建唯一投递记录，状态为 `pending`、`claimed`、`sent` 或 `skipped/failed`。在发送前原子认领，成功入队后记为 `sent`。`TurnInProgress` 是未入队的可重试情况，回到 `pending`；连接已断开则记失败并展示原 workflow 结果，不尝试向失效连接发消息。重连/重放读取记录，避免重复回发。由于 ACP 发送与数据库事务无法原子提交，进程在认领后崩溃会保留未确认状态；恢复时不盲目重发，以避免重复执行，记录需可诊断。事件落库前进程崩溃也可能丢失自动回传；本方案保证活跃进程内的投递与跨重放去重，不承诺跨崩溃的严格恰好一次送达。
5. 同连接多个 workflow 按首次终态顺序逐个投递，每次只发一个 prompt，待该 prompt 的 `TurnComplete` 再冲刷下一项。若用户 prompt 已被接受，自动投递继续等待该回合结束；完全同时的发送由现有 `prompt_lock` 排序，不虚构额外的优先级。会话切换、用户取消该 workflow、连接清理时，旧会话条目不跨会话投递。

`TurnComplete` 的生命周期处理和定时器冲刷已有顺序约束。实现时应把 workflow 冲刷放在同一个连接 worker 的终态处理后，再冲刷已排队 timer wake；已接受的用户 prompt 由发送锁保护。锁内最终校验负责解决“检查空闲后用户同时发送”的竞争。

## 结果提取与预算

名称和最终状态从合并后的 `WorkflowRun` 读取；投递记录里的证据正文只从被接受的终态帧及其后续有效终态修订提取。这条证据不写入发给模型的用户文本。`result_summary.trim()` 优先；`failed` 时回退到有效的 `last_event_detail.trim()`；两者都没有则仅报告状态未知详情。AIR 进度帧的 `summary` 可能被现有合并逻辑保留，不能在终态帧缺少文本时把它冒充最后总结。与 `current_phase` 相同的 `last_event_detail` 是进度名，不作为错误结论。Grok 的 `report.md` 读取函数暂不接入发送路径；如真实终态事件被证明只含节选，再单独接入经过 run id 校验的文件读取，并定义事件/文件一致性。

完整报告继续保留在 workflow 记录与历史展示中。回模型的结果文本上限设为 24 KiB UTF-8；超限时保留开头 8 KiB 与结尾 16 KiB，按字符边界截取，中间用明确的省略标记连接，以保留报告末尾的结论/下一步。状态、id、来源说明与末尾决策指令不计入这项结果截断。失败文本同样限长，日志只记长度/状态/id，不记录报告原文。实现时还需以模型请求预算测试校验该固定上限，不把 UI 的 512 KiB 文件上限直接用于 prompt。

## 自动消息与注意力设计

ACP 上这是一个普通文本 prompt，不能伪装成 system role。消息应在会话时间线显示为“Codeg · Workflow 结果”，保留稳定 `run_id`，并在实时 `UserMessage`、快照及历史解析中区分宿主来源与人工输入。跨端客户端仍能看到这条消息，重连后不会因乐观消息与回放产生双份；不发“用户刚发送消息”的频道通知。

发给模型的用户文本是一段英文结构。名称限长。技术状态只写 `completed` 或 `failed`，不附带结果正文，也不写 Run ID 或结果来源。完成与失败使用各自的行动条目，要求模型在本回合执行下一步，而不是只提出建议。

```text
[system notification]
Workflow: <name>
Status: completed

This run has ended. Use the outcome already in this conversation. Compare it with the user's goal and do the next required step in this turn.
- If the goal is met, summarize what was completed, then stop.
- If work remains, continue that work now. Do not stop at a proposal.
Do not restart this workflow merely because this notification arrived.
```

`failed` 的行动条目改为：说明失败原因；立即采取下一个有依据的步骤，若目标无法继续则总结阻塞并停止。

## 改动边界与验证

| 模块 | 计划改动 |
| --- | --- |
| `src-tauri/src/acp/session_state.rs` | 终态边沿、待投递项、会话切换清理及重复增量保护 |
| `src-tauri/src/acp/lifecycle.rs` | 订阅 workflow 低频事件；在 `TurnComplete`/空闲时调度冲刷 |
| `src-tauri/src/acp/manager.rs` | `WorkflowCompletion` 来源、串行投递、发送前检查和定时器顺序 |
| `src-tauri/src/acp/workflow_follow_up.rs` | 结果提取、预算与固定提示词的纯函数及代理策略 |
| `src-tauri/src/db/` | 唯一投递记录及状态迁移，防止连接重建和重放重复发送 |
| `src-tauri/src/acp/types.rs`、`src/lib/types.ts`、会话渲染层 | 宿主消息来源元数据及跨端/历史展示 |

重点验证：成功、失败、缺少结论、终态后补充摘要、`stopped` 修正为失败、进度摘要不得冒充终态结论、过期/重复 revision；终态到达于回合内和空闲时；并发用户输入、两个 workflow、timer wake；连接断开/重连、事件重放、进程认领后崩溃；机器消息的实时、快照、历史显示；长 Markdown、多字节截断及结果中的指令注入文本。白名单当前包含 Grok、Claude Code、Codex；OpenCode 在被宣告 `asyncTasks` 之前保持关闭。运行相关 Vitest、`pnpm rust test --features test-utils --lib workflow`、桌面与 server 模式的 `pnpm rust check`；本阶段仅形成方案，不修改运行代码。
