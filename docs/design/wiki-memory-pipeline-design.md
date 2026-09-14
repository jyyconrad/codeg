# 个人 Wiki 记忆流水线方案

| 字段 | 值 |
| --- | --- |
| 文档标题 | 个人 Wiki 记忆流水线方案 |
| 日期 | 2026-09-13 |
| 状态 | 已对齐产品选择，待实现 |
| 修订 | 2026-09-13：用 turn / session 记忆页 + 定时归纳替换 WeKnora 候选编译 |
| 受众 | 产品与 Codeg 工程师 |
| 仓库 / 分支 | `codeg` / `develop-20260909` |
| 应用版本基线 | `0.30.7` |
| 上游产品合同 | [个人工作 Wiki 与个人能力 Wiki 方案](./2026-09-12-personal-wiki-design.md) |
| 入库副本 | [docs/design/wiki-memory-pipeline-design.md](../../design/wiki-memory-pipeline-design.md)（`docs/superpowers/` 被 gitignore） |

## 1. 问题

当前工作台「工作」Tab 把来源清单当成工作内容：会话名重复、`ACP turn:` 截断句、`Unable to summarize…`，并被旧编译错误 `segment s16 returned more than 5 candidates` 挡住。ingest 只产一行「材料关于什么」；compile 按段抽候选，配额失败则整批不提交，vault 里没有可读的「做了什么」。

产品要的不是再放宽候选上限，而是换流水线：

1. ACP 一轮结束后立刻写成可读记忆：改了什么代码、实现了什么功能。
2. 对话真正完成后，用已有记忆（不够再回读会话）写成一篇对话页。
3. 定时任务再从这些记忆归纳项目工作与能力；项目文件夹按需读，不强制扫描。

## 2. 与 2026-09-12 方案的关系

**仍然有效：** 双 Wiki 目标（工作 / 能力）、一份 vault、Obsidian Markdown、ACP 成功轮冻结 `raw/`、项目绑定 `(db_instance_id, root_folder_id)`、外部导入格式、commit 的 `codeg-content` 区与 hash 冲突、证据角色不得由模型伪造、WikiWorker 不创建可见会话、不弹权限卡、无 bash/MCP/subagent。

**本稿替换：**

| 2026-09-12 | 本稿 |
| --- | --- |
| P6：ingest 一行摘要，compile 更新知识 | `turn_summary` 写记忆页；`session_rollup` 写对话页；`wiki_synthesize` 更新项目/能力 |
| §3 / compile skill：WeKnora 候选 → 匹配 related≠same → 合并 → Finalize | 取消候选数组、分段配额、`MAX_CANDIDATES_PER_SEGMENT` 作为生产失败条件 |
| 工作页用项目卡片 + 来源标题顶替笔记 | 工作页主列表是 `work/turns/`、`work/sessions/` |
| P2：导入与会话走同一套编译 | 导入仍冻来源、进资料 Tab；**不**进本期定时归纳 |
| 技能 `wiki-ingest` / `wiki-compile` | `wiki-turn-summary` / `wiki-session-rollup` / `wiki-synthesize` |

打开 Obsidian、撤回、原件下载、冲突 UI、OCR/PPTX 仍后置，与 2026-09-12 §1.1 一致。

## 3. 已锁定的产品选择

| 编号 | 选择 |
| --- | --- |
| D1 | 先每一轮 ACP turn 写记忆；之后 agent 以摘要为主，不够再回读整段 session |
| D2 | 工作页要有对话记忆，也要保留项目/职责/决策/成果/能力 Wiki |
| D3 | 定时归纳的输入是记忆页 + **按需**读已绑定项目根目录；不强制扫仓库；导入资料不进归纳 |
| D4 | 只绑已有完成事件：ACP 成功 `end_turn`、对话 `Completed`。不用空闲计时、不用离开页面猜测 |
| D5 | 严格按事件：`end_turn` → turn 页；`Completed` → session 页；未标完成就没有 session 页，定时任务仍可用 turn 页归纳 |
| D6 | 丢掉 WeKnora 候选流水线，改成三层任务 |
| D7 | 旧失败 compile（含 s16）留下的 `raw/` 与来源，按新任务重写成记忆页，不重跑候选、不把候选 JSON 当正文 |

## 4. 流水线

捕获保持现路径：ACP 成功 `end_turn` 在 `SessionState` 同锁冻结 `WikiTurnSnapshot`，`dispatch_run_settled` 入队持久化，写 `raw/sessions/<source-id>.md`，登记 `wiki_source`（已有 `conversation_id`、`folder_id`、项目绑定）。之后不再入队旧 `ingest` / WeKnora `compile`。

```text
ACP end_turn 成功
    → 冻 raw/sessions/<source-id>.md
    → 入队 turn_summary
    → 提交 work/turns/<slug>-<id>.md

对话状态变成 Completed
    → 入队 session_rollup
    → 读该 conversation 的 turn 页；不够再读 raw / 导出会话正文
    → 提交 work/sessions/<slug>-<id>.md

compile cron（默认 0 3 * * *）或「立即整理」
    → 入队 wiki_synthesize
    → 读新记忆页 + 已有工作/能力页；按需读绑定项目根
    → 提交 work/projects|areas|records|decisions|outcomes 与 capabilities/
```

| 任务 `kind` | 触发 | 模型主要读 | 模型写（经暂存提交） | 明确不做 |
| --- | --- | --- | --- | --- |
| `turn_summary` | ACP `end_turn` 冻结成功 | 这一轮 `raw/` + vault `AGENTS.md` | 一篇 `turn-summary` | 不改项目/能力页；不扫仓库 |
| `session_rollup` | `ConversationStatus::Completed` | 该对话 turn 页 + 对应 raw；按需会话导出 | 一篇 `session-summary` | `PendingReview` 不跑；不强制读全文 |
| `wiki_synthesize` | cron / `wiki_compile_now` | 新记忆页 + 已有 Wiki 页；按需项目根 | 项目/职责/记录/决策/成果/能力 | 不抽候选、不按段配额失败、不读未绑定目录 |

引擎每个 tick 领取顺序：`turn_summary` → `session_rollup` → `wiki_synthesize`。记忆页优先。某 `session_rollup` 的 `conversation_id` 上若仍有 queued/running `turn_summary`，本 tick 跳过该 session 任务。`wiki_synthesize` 同时最多一个（沿用现 compile 互斥）。

### 4.1 本期捕获范围

- **ACP：** 已有逐轮冻结，因此有 turn 页。
- **本地 Codeg Agent：** 本期不逐轮冻。对话 `Completed` 时宿主把会话正文写成 `raw/sessions/<id>.md`，`wiki_source.source_kind = local-session`，再跑 `session_rollup`。
- **聊天通道：** 现有实现会在 `end_turn` 时把对话标 `Completed`。允许同一轮既有 turn 页又有 session 页（session 是这一轮的汇总）。Delegate / Loop 仍按采集排除规则跳过。
- **导入文档 / 粘贴 / 导入本地会话目录：** 只冻来源、进资料 Tab。`session_import` **不得**再调用 `wiki_compile_now`。

### 4.2 命令与设置兼容

- HTTP/Tauri `wiki_compile_now` **保留名称**，语义改为入队 `wiki_synthesize`。
- 设置 JSON 新键：

```json
{
  "turn_summary": { "model_id": null, "prompt": null },
  "session_rollup": { "model_id": null, "prompt": null },
  "synthesize": { "enabled": true, "model_id": null, "prompt": null }
}
```

- 归纳开关与时刻：新设置写入 `synthesize.enabled`，cron 仍用 `compile_cron` / `next_compile_at`（不改列名）。读取旧 JSON 时：`synthesize.enabled` 缺省则抄 `compile.enabled`（缺则 true）。**不**把旧 ingest/compile 自定义提示词拷进新槽。
- GET 视图返回三份 builtin 全文：`turn_summary_builtin_prompt`、`session_rollup_builtin_prompt`、`synthesize_builtin_prompt`。
- 旧 `ingest` / `compile.prompt` 停止写入；读到则忽略。

## 5. 记忆文档与工作页

记忆页和归纳页分开。turn/session 是「做了什么」；项目/能力仍由 `wiki_synthesize` 从记忆归纳。

Vault 在现有目录上增加：

```text
work/turns/
work/sessions/
```

`initialize_vault` 创建这两个目录。新页面类型：

| `type` | 稳定路径 | 身份 |
| --- | --- | --- |
| `turn-summary` | `work/turns/{source_id}.md` | 与 `wiki_source.id` 1:1；`codeg_note_id` 宿主生成 |
| `session-summary` | `work/sessions/c{conversation_id}.md` | 与 `conversation_id` 1:1；再次 Completed 覆盖同一页 |

日期和可读标题放 YAML / 正文，不放路径。回填用「该路径是否已有文件」判断，禁止因 slug 变化写出第二份。同一 `source_id` 重试必须更新已有 turn 页。路径由宿主写入 payload，模型不得自造 `codeg_note_id` 或改 `raw/`。

### 5.1 Turn 页合同

YAML 至少包含：`title`、`type: turn-summary`、`tags: [type/turn-summary]`、`codeg_note_id`、`codeg_source_id`、`codeg_conversation_id`（可空）、`occurred_at`（有则写）、`sources`（指向 `sources/` 或 raw 阅读页，若尚无来源阅读页可只写 `codeg_source_id`）。项目绑定有则写 `projects` wikilink 或 `codeg_project_binding_id`。

正文必须是人能读的「这一轮做了什么」，例如：

- 改了哪些代码 / 模块（快照 `file_changes` 有则用，没有则写「未在快照中看到文件改动」）。
- 实现了什么、结论是什么。
- 结果是助手报告还是已核验；禁止把工具成功写成用户独立完成或能力精通。

标题禁止使用会话名原样、禁止 `ACP turn:` 前缀、禁止来源 UUID。语言跟材料走。长度由模型决定；宿主沿用叶子页软/硬上限（软警告、硬拒绝该页，不截断）。

快照无实质内容（空助手、全红、过滤跳过）可以不写 turn 页，来源保持 raw，job 记 `nothing_to_summarize` 成功。

### 5.2 Session 页合同

YAML：`type: session-summary`、`codeg_conversation_id`、链到下属 turn 页的 `turns` wikilink 列表。正文汇总「这次对话做了什么」。输入顺序：先读 turn 页；只有摘要不够、互相矛盾、或需要原文细节时再读 raw / 导出会话。没有 turn 页的本地会话：只读宿主写入的 `local-session` raw。

未 `Completed` 的对话不得有 session 页。

### 5.3 工作 Tab

- 按 `wiki_project_binding` 分组。卡片头：已有项目笔记用其 `title`，否则用目录名（沿用 `wikiProjectDisplayTitle`）。
- 主列表：该项目下的 turn / session 记忆（标题 + 摘要），**禁止**再渲染 `wiki_source` 标题列表。
- 无记忆页时显示空态说明，即使资料 Tab 里已有来源。
- 红条只取最近一次 **失败的 `wiki_synthesize`**（`kind === "wiki_synthesize"` 且 status failed）。不得展示旧 `compile` 的 `segment s16 returned more than 5 candidates`。
- 订阅 `wiki://job-changed`（引擎已 emit，工作台目前未听）。`turn_summary` 成功后列表刷新，不必离开页面。

能力 Tab 只浏览 `capabilities/`。归纳未跑时允许空，不拿来源充数。资料 Tab 仍是来源列表。任务 Tab 展示三种 kind，文案：回合摘要 / 对话汇总 / 工作归纳。「立即整理」只入队 `wiki_synthesize`。

工作 Tab 数据不要在前端扫整库 Markdown。新增命令 `wiki_list_memory_notes`：宿主扫描 `work/turns/`、`work/sessions/`，解析 YAML + 正文首段，返回 `{ rel, page_type, title, summary, occurred_at, conversation_id, source_id, project_binding_ids, codeg_note_id }`。`project_binding_ids`：YAML 里有 `codeg_project_binding_id` 则用它；否则用 `codeg_source_id` 查 `wiki_source.project_ids`。都没有则进未分组，不得因此去列来源标题。

## 6. 旧数据回填

失败 WeKnora compile **没有**提交工作页。留下的是 `raw/sessions/`、`wiki_source`、失败 `wiki_job`。s16 那批 ACP 来源按新记忆页重做。

引擎启动（wiki 已启用）执行一次幂等回填：

1. 将 status 为 queued/running/failed 的旧 `kind=compile` 标 `cancelled`，`error_code=superseded`。禁止按原错误重试。
2. 每个 `source_kind=acp-turn`、有 raw、尚无对应 turn 页的来源入队 `turn_summary`（去重键见 §7）。包括当时挂在失败 compile 上的来源。
3. 这些来源的 `conversation_id` 若对话已是 `Completed` 且尚无 session 页，再入队 `session_rollup`。
4. 不把旧候选 JSON、`wiki_source_segment` 分析缓存、ingest 一行摘要当作记忆正文。ingest 摘要最多当线索；正文以 raw 为准。
5. 回填只入队，由 tick 顺序消化，不在启动时同步打满模型。
6. `wiki_compile_input` 旧合同版本作废。新合同 `codeg.wiki.synthesize.v1` 记录「哪篇记忆页内容哈希已被归纳」。回填后的记忆可以重新进入 `wiki_synthesize`。

回填期间工作页可以短暂为空，但不得再出现来源标题墙和旧 compile 红条。

## 7. 任务去重与消费登记

| kind | `dedupe_key` | 成功登记 |
| --- | --- | --- |
| `turn_summary` | `turn_summary:{source_id}:{raw_hash}` | output_manifest 含 `rel`、`codeg_note_id`；`wiki_contribution` 记 source→turn 页。列表用的 `source_title` / `source_summary` 从该页标题和首段回填 |
| `session_rollup` | `session_rollup:{conversation_id}` | 同一对话覆盖同一 session 页 |
| `wiki_synthesize` | 定时：`vault_id + scheduled_for_utc`；手动：调用方 request UUID | output_manifest 列出消费的记忆 `{rel, content_hash}` 及提交的归纳页 |

活跃任务去重：相同 `dedupe_key` 已有 queued/running 则不新建。成功后同一 raw_hash 不再重做 turn 页，除非用户显式重试该 job。

`wiki_synthesize` 下一批输入 = `work/turns/` 与 `work/sessions/` 中内容哈希未出现在最近一次成功归纳消费清单中的页面，加上这些页面链到的已有项目/能力页。没有新记忆时，定时任务仍可创建一个 succeeded、消费为空的 job，不调用模型。

## 8. 工具、沙箱、Skill

三层都走 WikiWorker：`PermissionPolicy::AutoAllow`，无 bash、无 MCP、无 subagent、无 plan。写只进当前 job staging。提交仍走 `commit.rs` 的 before_hash / `codeg-content` 拼接。

| 任务 | 读根 | 写根 |
| --- | --- | --- |
| `turn_summary` | vault（实际需要 raw 该文件 + `AGENTS.md`） | staging |
| `session_rollup` | vault（该对话 turn 页 + raw；本地会话则含导出 raw） | staging |
| `wiki_synthesize` | vault 的 `work/`、`capabilities/`、`knowledge/`、索引、`AGENTS.md`、`sources/`；**附加**本批记忆绑定到的项目根目录 | staging |

`FsAccessPolicy::wiki_worker` 扩展为可附加只读根。空根表示拒绝，不得退回 unrestricted。项目根只读：禁止写入项目树、禁止把项目文件当 Wiki 页提交。附加根必须是 `wiki_project_binding` 对应的 canonical 目录。拒绝 `.git/`、`.obsidian/`、`originals/`、凭证路径（沿用 `DENIED_SEGMENTS`）。宿主在 payload 里给出这些根路径字符串，**不**把 `ls`、diff、文件树塞进提示词。Agent 不读项目目录不算失败。

`WikiFsPolicy` 增加：

- `ALLOWED_PAGE_TYPES`：`turn-summary`、`session-summary`
- `page_type_matches_rel` / `is_allowed_commit_rel`：`work/turns/`、`work/sessions/`

Skill 文件：

- `src-tauri/agent-skills/wiki-turn-summary/SKILL.md`
- `src-tauri/agent-skills/wiki-session-rollup/SKILL.md`
- `src-tauri/agent-skills/wiki-synthesize/SKILL.md`

从 Worker 目录下架 `wiki-ingest`、`wiki-compile`。设置「恢复默认」读新 skill。`disable-model-invocation: true` 保持。模型循环上限：turn 8、session 16、synthesize 24（可与现 `WIKI_COMPILE_MAX_TURNS` 分开命名）。

`wiki-synthesize` **不得**要求返回候选数组。产出是暂存区内的 Markdown 页提案；宿主按现有 YAML/`type`/wikilink/叶子长度校验后提交。证据角色规则（不能把 agent 行为写成用户精通）仍由宿主在归纳提交时校验。

Turn/session：**宿主模板套正文**。模型只返回 JSON `{ title, body, warnings }`（schema 分 `codeg.wiki.turn_summary.v1` / `codeg.wiki.session_rollup.v1`）。宿主写入 YAML、`codeg_note_id`、路径和 `codeg-content` 区。模型不得自写整页 front matter。`wiki_synthesize` 仍用现有多页 Markdown 提案（模型写完整页，宿主校验 YAML/`type`/链接/长度）。

## 9. 完成事件挂钩

1. **Turn：** 保持 `try_freeze_wiki_snapshot` + `enqueue_persist`。持久化成功后入队 `turn_summary`，不再入队 `ingest`。
2. **Session：** 在对话状态写入 `Completed` 的唯一入口挂钩（`conversation_service::update_status` 成功且新状态为 Completed）。不要只在前端点完成；也不要另开一条只听事件、不写库的旁路。Wiki 关闭、采集排除的 agent/目录、Delegate/Loop：跳过。
3. **顺序：** 若该 `conversation_id` 仍有 queued/running 的 `turn_summary`，`session_rollup` 保持 queued，引擎不得先领取它。聊天通道同一 `end_turn` 既冻 turn 又标 Completed 时，先写 turn 页再写 session 页。
4. ACP `end_turn` → `PendingReview` **不是** session 完成，不入队 `session_rollup`。
5. 本地 Agent `Completed` 且没有任何 acp-turn 来源：先写 `local-session` raw 并登记 `wiki_source`，再入队 `session_rollup`。导出失败则 job 失败可重试，不得在没有 raw 时让模型凭空写 session 页。

## 10. 错误处理

沿用：临时失败最多自动重试 3 次（1/5/15 分钟）；认证、schema、路径、伪造 id 不自动重试；30 分钟预算；vault 提交互斥。

| 失败 | 用户可见 | 其它层 |
| --- | --- | --- |
| `turn_summary` | 该轮不出现在工作页；任务 Tab 可重试 | raw 保留；不回退到来源标题列表 |
| `session_rollup` | 无对话页 | turn 页照常可读 |
| `wiki_synthesize` | 工作页红条 = 这次归纳错误 | 记忆页照常可读 |
| 单页超硬上限 | 只拒该页 | 不得因此把整批 synthesize 标失败，除非提交清单为空且必须处理的页全被拒 |

旧 `compile` 失败记录回填时取消，工作页查询失败任务时过滤 `kind=wiki_synthesize`。

## 11. 设置与 i18n

设置页三个提示词槽：回合摘要、对话汇总、工作归纳，各有恢复默认。整理开关绑定 `synthesize.enabled`。采集排除项不变。

i18n（`Wiki.*`）增加 job kind 文案、记忆空态、synthesize 失败红条；删除或不再使用「来源计数当作工作内容」的主列表文案。工作卡片的 `sourceCount` 改为记忆条数。

## 12. 测试

不把「模型文笔」当通过条件。通过条件：记忆页在、工作页读记忆页、旧候选链不能把整页打挂。

**Rust（`cargo test --lib wiki::` 及完成事件挂钩）**

- ACP `end_turn` 冻结成功 → 入队 `turn_summary`，不入队 `ingest`/`compile`。
- `Completed` → `session_rollup`；`PendingReview` 不入队。
- 同一对话仍有 queued/running `turn_summary` 时，引擎不领取该 `session_rollup`。
- 同一 `source_id+raw_hash` 不建第二份 turn 页；同一 `conversation_id` 不建第二份 session 页。
- 启动回填：无 turn 页的 acp-turn 入队；旧 `compile` 被 cancelled/superseded。
- `wiki_synthesize` payload 含记忆 `rel`，不含 raw 切片和 `candidates` 数组。
- 项目根只出现在 synthesize 读根；turn/session 沙箱没有它。
- synthesize 失败不回滚已提交的 turn 页。
- `turn-summary` / `session-summary` 只能提交到 `work/turns/`、`work/sessions/`。
- 导入路径不再调用 `wiki_compile_now`。
- 叶子超长只拒该页。

**前端**

- 工作 Tab fixture：有记忆页则按项目显示标题+摘要，不渲染来源标题。
- 无记忆、有来源：空态，不出现「21 个来源」。
- 红条只认 `wiki_synthesize` 失败。
- 能力 Tab 无 capability 笔记时为空。
- 任务 kind 文案与「立即整理」只打 synthesize。
- `wiki://job-changed` 后工作列表刷新（可用 mock 事件）。

**装包后手工**

- 回填结束后，原 21+9 条来源变成可读「做了什么」，s16 红条消失。
- 新 ACP 一轮结束，工作页很快出现该轮记忆，不等 cron。
- 对话标完成后出现 session 页。
- 立即整理才更新项目/能力；agent 可以不读仓库。

## 13. 主要改动落点

| 区域 | 文件 |
| --- | --- |
| 入队 / 回填 / tick | `src-tauri/src/wiki/source.rs`、`engine.rs`、`worker.rs` |
| 完成事件 | `db/service/conversation_service.rs` 或 ACP lifecycle 旁路；禁止只在前端点完成 |
| 三任务执行 | 新 `wiki/turn_summary.rs`、`wiki/session_rollup.rs`；`compile.rs` 去掉候选四步，改为 `synthesize`（可改名） |
| 页面策略 | `vault.rs`、`fs_policy.rs`、`commit.rs` |
| 工具 | `FsAccessPolicy::wiki_worker`、`NativeTurnTools`、`wiki/llm.rs` |
| Skill | 三份新 SKILL.md；`builtin_skills.rs`；下架 ingest/compile |
| 设置 | `wiki/settings.rs`、`wiki-settings.tsx`、`wiki-types.ts` |
| 工作台 | `wiki-work-view.tsx`、`wiki-jobs-view.tsx`、`wiki-capabilities-view.tsx`、`api.ts`、i18n |
| 导入 | `session_import.rs` 去掉自动 compile |
| 列表 API | 新 `wiki_list_memory_notes` |

`wiki_source_segment` 表保留但不作为生产输入。不在本期做 schema 删除迁移。

## 14. 非目标

- 空闲超时或「离开工作台」触发 session 页。
- 把导入 PDF/目录自动纳入 synthesize。
- 本地 Agent 逐轮冻结（只在 Completed 做 session 页）。
- 恢复 WeKnora 候选配额或把 5/80 当成产品限制。
- 打开 Obsidian、撤回、原件下载、冲突解决 UI。
- 强制扫描 git log / 全仓库作为归纳输入。
- 把旧候选 JSON 转成记忆页。

## 15. 验收口径

同时满足：

1. 工作 Tab 主列表是 turn/session 记忆，不是来源标题墙。
2. ACP 成功一轮后，无需等到 cron，即可读到该轮「做了什么」。
3. 只有对话 `Completed` 才有 session 页。
4. 定时或立即整理才更新项目/能力；不读项目目录也可以成功。
5. 旧 s16 失败来源经回填成为记忆页；工作页不再显示该错误。
6. 新代码路径上不再出现 `returned more than 5 candidates` 这种生产失败。
