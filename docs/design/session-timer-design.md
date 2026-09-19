# 会话定时器（set_session_timer）

| 字段 | 值 |
| --- | --- |
| 文档标题 | 会话定时器 |
| 日期 | 2026-09-20 |
| 状态 | Approved — 方案 1；授权实现 |
| 受众 | Codeg 工程师 |
| 上游 | 现有 `codeg-mcp` companion 注入、`CompanionFeatures::allows_tool` 白名单、`ConnectionManager::send_prompt_linked`、空闲扫描 `sweep_idle` |

---

## 1. 目标与边界

Agent 调用工具 `set_session_timer`，为**当前会话对应的 ACP 连接**预约一次唤醒。到期后 host 向**同一连接**注入一条新提示，开启新回合。最长 30 分钟。只和该连接/会话绑定，不能指定或唤醒其它会话。

以 **tools** 形式接入：MCP companion（codex、grok 等）与 Codeg Agent 原生 companion 共用 `tool_schema.json`。以 **白名单** 适配全部能收到 codeg-mcp 的 agent：工具名必须出现在 `CompanionFeatures::allows_tool` 中才会 `tools/list` / `tools/call`；未知名一律拒绝。

| # | 目标 | 验收 |
| --- | --- | --- |
| T1 | 工具 `set_session_timer` | `seconds` 1–1800；可选 `reason`；`cancel_on_user_message` 默认 true |
| T2 | 每连接一条定时器 | 新调用覆盖旧定时器，返回 `replaced` |
| T3 | 到期注入新提示 | 当前无回合则 `send_prompt_linked`；有回合则等 `TurnComplete` 后再注入 |
| T4 | 用户发言默认可取消 | 默认取消；模型可传 `cancel_on_user_message: false` 让到期仍唤醒 |
| T5 | 连接保活到到期 | 空闲扫描不拆有未到期定时器的连接；主动断开或进程退出则作废 |
| T6 | 全 agent 白名单 | MCP 可达的内置/自定义 agent + 原生 `codeg_agent`；pi 仍不注入 MCP |

**不做（首期）：**

- 持久化、应用重启后恢复、断线 resume
- 跨会话、全局、cron
- 独立 cancel 工具（覆盖即取消；用户发言默认可取消）
- 设置页开关（`timer` 组始终开启，不进 `set_codeg_mcp_tool_group`）
- 阻塞等待（同一回合 sleep）
- 每个 agent 各写一套原生桥（grok `ask_user_question` 那种）

---

## 2. 关键决策

| # | 决策 | 理由 |
| --- | --- | --- |
| K1 | 到期注入新提示，不阻塞当前回合 | 已选方案；工具立即返回 ack |
| K2 | 按 `connection_id` 绑定，每连接最多一条 | 「只和会话关联」；连接是 ACP 会话的运行时身份 |
| K3 | 新调用覆盖旧定时器 | 比多定时器简单；模型重设即换时间 |
| K4 | `cancel_on_user_message` 默认 true，可 false | 用户已选：默认取消，模型可选择到期仍唤醒 |
| K5 | 首期保活连接，不持久化 | 空闲扫描默认 3 分钟会拆连接；有未到期定时器则跳过扫描。断开/退出作废 |
| K6 | 不复用 `background_outstanding` | 那是后台 shell 心跳，有 keepalive 窗口语义；定时器用独立 deadline |
| K7 | `timer` 功能组始终开启 | 白名单接入所有 MCP agent；`snapshot_companion_features` 置 `timer: true`。`Default` 仍全 false，避免单测误伤 |
| K8 | 唤醒提示带固定前缀 `[session timer]` | 模型能识别这是预约唤醒；UI 当普通 user 回合展示 |
| K9 | 唤醒发送走 `PromptSource::SessionTimer` | 避免唤醒自己把自己取消 |
| K10 | 唤醒失败（TurnInProgress）排队，不丢 | 到期时若 `turn_in_flight`，写入 `queued_timer_wake`，`TurnComplete` 后刷新 |
| K11 | 工具无 `session_id` 参数 | 父连接来自 companion token / `CompanionRuntime.connection_id`，不能瞄其它会话 |
| K12 | Plan mode 允许该工具 | 只预约 host 唤醒，不写盘；加入 `companion_ok_in_mode` |
| K13 | 不进 MCP 设置分组列表 | 与 `tasks` 一样：不是用户开关。始终注入 companion（只要 agent 能投递 MCP） |

---

## 3. 工具契约

名称：`set_session_timer`

`inputSchema`：

```json
{
  "type": "object",
  "required": ["seconds"],
  "properties": {
    "seconds": {
      "type": "integer",
      "minimum": 1,
      "maximum": 1800,
      "description": "Seconds to wait before waking this same session. Maximum 1800 (30 minutes)."
    },
    "reason": {
      "type": "string",
      "description": "Optional note included in the wake prompt so you remember why you waited."
    },
    "cancel_on_user_message": {
      "type": "boolean",
      "description": "If true (default), a later user message on this session cancels the timer. If false, the wake still fires after the wait."
    }
  }
}
```

**描述（写入 schema，供模型阅读）：** 预约当前会话在指定秒数后被 host 重新唤醒。立即返回，不阻塞本回合。到期后 host 向本会话注入一条唤醒消息并开新回合。最长 1800 秒。同一会话只能有一条未到期定时器；再次调用会替换。默认情况下用户之后发的消息会取消定时器；若要边聊边等，传 `cancel_on_user_message: false`。不要用它等待其它会话或超过 30 分钟的工作。

**解析规则**（`parse_timer_args`）：

- `seconds` 必须是 JSON 整数，或只含十进制数字的字符串。范围 `[1, 1800]`。否则错误。
- 浮点、布尔、缺失、0、负数、1801+：错误，不改现有定时器。
- `reason`：可选字符串；trim 后空则当 `None`。非字符串忽略为 `None`。
- `cancel_on_user_message`：缺省或 JSON null → `true`。JSON boolean 按其值。其它类型 → 错误。

**成功返回**（companion `content[0].text`，与其它 companion 工具一致）：

- 未覆盖：`Timer set: wake this session in {seconds}s.`
- 覆盖：`Timer set: wake this session in {seconds}s. Previous timer replaced.`

`structuredContent`：

```json
{
  "ok": true,
  "timer_id": "<uuid>",
  "seconds": 120,
  "cancel_on_user_message": true,
  "replaced": false
}
```

**失败：**

- 参数错误：MCP `-32602` / 原生 `invalid_args`，文案可给模型改。
- 连接不存在：软失败 `ok: false`，`error: "connection_not_found"`（与 `get_session_info` not_found 同类，不当成协议崩）。

---

## 4. 唤醒文案

`wake_prompt_text(spec)` 必须稳定（测试锁定原文）。有 reason：

```
[session timer] Scheduled wake.

Waited: {seconds} seconds.
Reason: {reason}

This is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on.
```

无 reason：去掉 `Reason:` 行及其后空行，保持：

```
[session timer] Scheduled wake.

Waited: {seconds} seconds.

This is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on.
```

注入为单个 `PromptInputBlock::Text`。走 `send_prompt_linked`，会进 transcript / UI，像用户回合。

---

## 5. 生命周期

```
set_session_timer
    → 取消该 connection_id 上旧任务（CancellationToken）
    → 登记 SessionTimer { timer_id, spec, cancel_on_user_message }
    → SessionState.session_timer_deadline = now + seconds
    → spawn sleep(seconds)；取消则退出
    → 到期：map 中 timer_id 仍匹配才 fire
         若 turn_in_flight → queued_timer_wake = spec；清 deadline（不再挡扫描？见下）
         否则 send_prompt(PromptSource::SessionTimer)
    → 成功注入后清 registry + deadline + queue

用户 PromptSource::User（send_prompt_linked 非常驻唤醒）
    → 若当前定时器 cancel_on_user_message → 取消任务、清 registry/deadline/queue

disconnect / 连接线程结束
    → cancel_by_parent(connection_id)

TurnComplete
    → 若 queued_timer_wake 仍在且 turn_in_flight 已清 → send_prompt(SessionTimer)
```

**排队时的扫描：** 已到期、正在等回合结束时，连接处于 `Prompting` / `turn_in_flight`，扫描本就会跳过。不必再靠 deadline。到期后立刻清 `session_timer_deadline`，只留 `queued_timer_wake`。

**用户消息取消 vs 排队唤醒：** 用户在排队窗口发言 → `PromptSource::User`。若 flag true，丢掉 `queued_timer_wake` 且取消（此时 sleep 任务多半已结束）。若 flag false，用户回合会 `TurnInProgress` 被拒或等当前回合结束；当前回合结束后若 queue 仍在则仍会唤醒。用户成功开新回合时 `turn_in_flight` 为真，到期 fire 只会排队；用户回合 `TurnComplete` 后若未被取消则唤醒。符合「false 则到期仍唤醒」。

**覆盖：** 新 `set_session_timer` 取消 sleep、丢掉 queue、换新 deadline。

**单调时钟：** `tokio::time::sleep(Duration::from_secs(seconds))`，不拿墙钟比 `fire_at`，避免改系统时间提前触发。

---

## 6. 空闲扫描

`SessionState` 增加：

- `session_timer_deadline: Option<DateTime<Utc>>` — 未到期则跳过 `sweep_idle`
- `queued_timer_wake: Option<SessionTimerSpec>` — 到期但回合未结束

`has_pending_session_timer(now) -> bool`：`deadline` 为 Some 且 `deadline > now`。

`sweep_idle` 在 `has_active_background_work` 之后、比 `last_activity_at` 之前调用它。有未到期定时器则不拆。

不要增加 `background_outstanding`。Deadline 只在 `set_timer` / 取消 / 到期时更新。

---

## 7. 全 agent 白名单

接入路径只有一条：companion 工具名白名单 + 现有注入。

1. `tool_schema.json` 增加 `set_session_timer`。
2. `CompanionFeatures.timer: bool`；`parse("timer")`；`allows_tool("set_session_timer") => self.timer`。其它名字仍 `_ => false`。
3. `CompanionFeatureFlags.timer`；`companion_features_arg` 在 `taskboard` 之后追加 `"timer"`。
4. `snapshot_companion_features` **恒为** `timer: true`（与 host_tools、settings 无关）。
5. `inject_codeg_mcp`：因 timer 常开，MCP 可达的 agent **总会**尝试注入 companion（二进制缺失则仍跳过并打日志）。
6. `agent_delivers_wire_mcp`：pi 仍不注入。
7. `CodegAgent`：`load_companion_defs` / `call_companion_tool` 同一 schema；不走 MCP stdio。
8. 无 per-agent 特殊大小写或别名。

`allows_tool` 默认拒绝未知名，这就是「白名单」。

设置 UI 的 `tool_groups` **不**增加 `timer` 行（没有开关）。

---

## 8. 模块与接口

新建 `src-tauri/src/acp/session_timer.rs`（对照 `session_info.rs` / `question.rs`）：

```rust
pub const MAX_TIMER_SECONDS: u32 = 1800;
pub const WAKE_PREFIX: &str = "[session timer]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTimerSpec {
    pub seconds: u32,
    pub reason: Option<String>,
    pub cancel_on_user_message: bool,
}

#[derive(Clone, Debug)]
pub struct SessionTimerAck {
    pub ok: bool,
    pub timer_id: Option<String>,
    pub seconds: u32,
    pub cancel_on_user_message: bool,
    pub replaced: bool,
    pub error: Option<String>,
}

pub fn parse_timer_args(args: &serde_json::Value) -> Result<SessionTimerSpec, String>;
pub fn wake_prompt_text(spec: &SessionTimerSpec) -> String;
pub fn render_timer_ack(ack: &SessionTimerAck) -> serde_json::Value; // MCP content 形状

#[async_trait::async_trait]
pub trait SessionTimerAccess: Send + Sync {
    async fn set_timer(&self, parent_connection_id: &str, spec: SessionTimerSpec) -> SessionTimerAck;
    async fn cancel_by_parent(&self, parent_connection_id: &str);
}
```

`ConnectionManager` 增加与 `pending_questions` 一样、可 `clone_ref` 共享的：

```rust
session_timers: Arc<Mutex<HashMap<String, LiveSessionTimer>>>,
```

`LiveSessionTimer { timer_id, spec, cancel: CancellationToken, join: JoinHandle<()> }`。

生产实现 `ConnectionManagerTimerLookup { manager, db }` 放在 `manager.rs`，与 `ConnectionManagerQuestionLookup` 并列。`set_timer` 需要 `AppDatabase` 才能 fire 时 `send_prompt_linked`。测试可用内存 stub。

`DelegationInjection` 增加 `timers: Arc<dyn SessionTimerAccess>`。`run_connection` 清理与 `cancel_questions_by_parent` 一样调用 `timers.cancel_by_parent`。

Broker：

```rust
pub struct BrokerSetTimerRequest {
    pub token: String,
    pub seconds: u32,
    pub reason: Option<String>,
    pub cancel_on_user_message: bool,
}
// BrokerMessage::SetTimer(...)
```

Companion 在 `tools/call` 里解析参数后 round-trip。Listener 用 token 解析 `parent_connection_id`，再 `timers.set_timer`。

`send_prompt_linked_with_message_id` 增加内部来源，避免再加一长串公开参数：

```rust
enum PromptSource { User, SessionTimer }
```

仅 manager 内部。公开 API 默认 `User`。Timer fire 走内部路径。`User` 在占用 turn gate 之前执行取消逻辑。

---

## 9. UI

- `CODEG_MCP_WORKBENCH_TOOLS` 增加 `set_session_timer`
- `tool-call-normalization.ts`：`/[^a-z0-9]set_session_timer$/` → `set_session_timer`
- `CodegMcpToolCard`：一行「Wake in {seconds}s」；有 reason 则带上
- 10 份 locale 的 `Folder.chat.codegMcpTool`：`setSessionTimer` / `setSessionTimerNoSeconds`

不做独立设置页、不做 composer 按钮。

---

## 10. 测试要点

领域：`parse_timer_args` 边界；`wake_prompt_text` 锁定；覆盖 / 取消 / 每连接一条。

扫描：有未来 deadline 的连接不被 `sweep_idle` 拆；过期 deadline 不挡扫描。

Fire：空闲连接注入带 `[session timer]` 的 prompt；`turn_in_flight` 时排队，`TurnComplete` 后注入；`PromptSource::User` 且 flag true 则取消；flag false 则保留；`SessionTimer` 来源不取消。

Companion：`allows_tool` 仅当 `timer`；`snapshot` 恒 true；`features_arg` 含 `timer`；未知工具仍拒绝。

前端：canonical 名；卡片展示 seconds。

---

## 11. Open Questions

无。产品选择已锁定。
