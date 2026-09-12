# 个人 Wiki 实现进度与待完善

| 字段 | 值 |
| --- | --- |
| 文档标题 | 个人 Wiki 实现进度与待完善 |
| 日期 | 2026-09-12 |
| 状态 | 主干可试用，**未达到方案 v1 验收** |
| 分支 / 提交 | `feat/personal-wiki` / `64422ca2` |
| 对照基线 | `develop-20260909` |
| 方案合同 | 本地 `docs/superpowers/specs/2026-09-12-personal-wiki-design.md`（该目录在 `.gitignore`，不入库） |
| 事实口径 | **目标**以方案 §1–14 为准。**实现**以本分支代码为准。未完成项不得写成已交付 |

本文只记当前代码做到哪、还差什么。页面合同、采集规则、Worker 边界仍以方案正文为准，不在这里改写。

未完成 **R1–R4** 时，发布说明不要写「个人工作 Wiki / 个人能力 Wiki 已完成」。当前对外口径应是：可试用的知识采集与阅读主干。

## 1. 当前结论

打开 **设置 → 个人 Wiki**（默认关闭）后，ACP 成功轮次可以冻结入库，外部 Markdown / TXT / 文本 PDF / DOCX 可以导入，工作台可以浏览 vault 里的 Markdown，任务页可以立即整理、重试、取消。

还不能按方案验收为「个人工作 Wiki / 个人能力 Wiki」：

- 项目身份表已建，编译未使用。
- 能力证据只改 YAML 字样，模型正文仍可能写出个人实践结论。
- 撤回、原件下载、冲突处理未做。
- 「工作」「能力」两页仍是目录树，不是项目列表和证据视图。

图谱、Canvas、向量检索、RAG 明确后置，不是本轮缺口。

## 2. 已经可用

| 能力 | 行为 | 代码落点 |
| --- | --- | --- |
| 总开关 | 默认关闭；启用后才采集、导入、领取新任务 | `src-tauri/src/wiki/settings.rs`，`src/app/settings/wiki/` |
| ACP 采集 | 首次有效 `end_turn` 同锁冻结快照，脱敏后写 `raw/sessions/`；不创建可见会话、不弹权限卡 | `session_state.rs` `try_freeze_wiki_snapshot`，`wiki/source.rs` |
| 工作台入口 | 侧栏 / 底栏「个人 Wiki」；全部 / 工作 / 能力 / 资料 / 任务 | workbench route `wiki`，`src/components/wiki/` |
| 全部页 | 递归浏览 vault Markdown；`[[wikilink]]` 库内跳转；默认隐藏 `raw/` | `wiki-all-view.tsx`，`wiki_vault_tree` / `wiki_vault_read` |
| 外部导入 | 文件多选或粘贴正文；字节哈希去重；`request_id` 幂等；部分提取需接受后才 `ready` | `wiki/import.rs`，`document_extract/` |
| 任务 | 立即整理、失败重试、未提交取消 | `commands/wiki_engine.rs`，`wiki-jobs-view.tsx` |
| 编译页 | 宿主模板覆盖项目 / 职责 / 记录 / 决策 / 成果 / 能力 / 方法与概念 / 来源阅读页 | `wiki/compile.rs`，`wiki/commit.rs` |
| 日记 | 按来源 `occurred_at` 的 vault 时区日期；缺发生时刻不写到整理当天 | `wiki/compile.rs` |
| 证据压级 | 来源全是参考资料时，宿主把能力页 YAML 的 `practice_*` 换成 `knowledge_only` | `sanitize_capability_evidence` |
| 默认路径 | vault：`$CODEG_HOME/wiki` → `$CODEG_DATA_DIR/wiki` → `~/.codeg/wiki`；原件在 vault 外 `wiki-state/originals/` | `wiki/paths.rs` |

单测口径（记录时）：在 `src-tauri/` 执行

```bash
cargo test --no-default-features --features test-utils --lib wiki
```

通过 86 项。这只覆盖 Wiki 库内测试，不代替方案 §14 用户场景和桌面 / 服务器全量门禁。

## 3. 分阶段切片现状

相对 `feat/personal-wiki`（`64422ca2`），不是对方案合同的改写。

| 切片 | 方案目标 | 现状 |
| --- | --- | --- |
| PR1 来源基础与 ACP 采集 | 同锁冻结、稳定 `run_id`、vault/source/job、过滤、确定性 raw | **主干已合入。** 入队仍是进程内 `tokio::spawn`，不是 durable outbox；过滤跳过目前主要打日志 |
| PR2 外部导入 | 受控上传 / 粘贴、四种格式、哈希版本、原件、覆盖预览 | **主干已合入。** 撤回、纠错提取、按 `source_id` 下载原件、版本关联 UI 未做 |
| PR3 Worker 与提交 | 工具沙箱、暂存提案、校验提交、分段、恢复 | **骨架已合入。** 生产编译以 JSON 一轮调用为主；取消 / 超时主要改库状态；长文跨段归并未按方案分批 |
| PR4 工作与能力编译 | 项目映射、四步编译、证据规则、入口与日记 | **部分合入。** 宿主模板和日记已有；`wiki_project_binding` 未使用；工作 / 能力产品视图仍是目录树 |
| PR5 设置与工作台 | 四视图、导入标注、冲突、Obsidian 入口、i18n | **浏览主干已合入。** 冲突处理、系统打开、撤回、任务事件刷新未做 |

相关提交（旧 → 新）：

1. `a579feb7` 内置 ingest / compile skill
2. `86c02c69` 设置页与工作台视图
3. `20cdf7a1` ACP 快照、来源、vault、设置 API
4. `fcdfa844` MD/TXT/PDF/DOCX 提取器
5. `51a8995d` 接通提取并持久化快照
6. `1fb1acf9` 粘贴与文件导入
7. `0d075408` WikiWorker、暂存提交、立即整理
8. `2176be16` 导入字段合入编译测试、接通任务动作
9. `d98548f7` 「个人 Wiki」菜单与 vault 全文浏览
10. `b68dee58` 工作 / 能力页合同、日记、来源页
11. `64422ca2` 评审修补：捕获、恢复、证据 YAML 压级

## 4. 必须继续完善

未完成则不能按方案 v1 验收。编号稳定，后续实现或关闭时只改「状态」列，不改编号。

**阻塞产品表述**（R1–R4 未完成时，不要对外宣称双 Wiki 已交付）：

| 编号 | 状态 | 缺口 | 方案 | 代码线索 | 建议验收 |
| --- | --- | --- | --- | --- | --- |
| R1 | **未做** | 项目身份未接通。`wiki_project_binding` 已建表，采集和编译不写不读。同名仓库、worktree 与根项目可能混在一起 | §4.1、§10.1 | `db/entities/wiki_project_binding.rs`；migration `m20260912_000001_wiki.rs`；除实体注册外无读写 | ACP 来源按 `(数据库实例, root_folder_id)` 落到同一项目页；同名不同根目录不合并 |
| R2 | **部分** | 能力证据仍可能被模型正文带偏。宿主只替换 YAML 里的 `evidence_level` 字样；`page_proposals` 里的实践证据段落、actor、`verification_status` 未按 §4.2 逐项校验再提交 | §4.2、§9.3、§11.3 | `wiki/compile.rs` `sanitize_capability_evidence` | 参考资料导入不得写出个人实践结论；代理 / 团队动作无 `personal_role` 不得升格 |
| R3 | **未做** | 纠正与隐私闭环未做。无撤回来源、无按 `source_id` 下载原件、无冲突查看 / 解决 API 与界面。冲突文件只落到 `wiki-state/conflicts/`。`wiki_reextract` 已有，纠错提取（新 raw）未做 | §12.3、§13、§11.3.5 | `commands/wiki.rs` 无 withdraw / original download / conflict；`wiki-page.tsx` 无对应操作 | 撤回后旧标注不再支撑新结论；用户能下载原件核对；部分提交冲突时暂停该 vault 后续写入直至处理 |
| R4 | **未做** | 工作 / 能力工作台仍是文件树。没有项目列表、证据缺口、案例、下一次实践 | §12.2、§1 | `wiki-work-view.tsx`、`wiki-capabilities-view.tsx` 均包 `WikiNoteBrowser`，前缀分别为 `work`、`capabilities` | 「工作」「能力」能按类型 / 证据浏览，而不是只打开目录 |

**运行可靠性与体验**（不补齐则试用不稳定，或与设置文案不符）：

| 编号 | 状态 | 缺口 | 方案 | 代码线索 | 建议验收 |
| --- | --- | --- | --- | --- | --- |
| R5 | **部分** | Worker 与方案中的工具沙箱不一致。`NativeTurnTools::wiki_compile` 已定义且有单测，生产编译基本走无工具 JSON 调用；长文候选未按批次归并 | §9.2、§11.1–11.2 | `agent/model/turn.rs` `wiki_compile`；生产路径在 `wiki/compile.rs` / `wiki/llm.rs` | 编译只读清单内笔记与 raw 分段，只写当前 job 暂存；超长来源分段覆盖可见且不静默丢段 |
| R6 | **部分** | 运行控制偏库状态。取消把 job 标 `cancelled`，30 分钟超时把 running 标 `failed`；在飞模型调用没有取消令牌。重试只把 failed 改回 queued，1/5/15 分钟退避未执行 | §10.2 | `wiki_service::cancel_job` / `retry_job`；`engine.rs` `expire_overdue_jobs` | 取消或超时后不再提交文件；可重试错误按退避排队 |
| R7 | **部分** | 设置与队列语义不完整。换 `vault_path` 不暂停在飞任务；总开关关闭不中断已发出的模型调用。「待分析」已排除进入 `wiki_compile_input` 的 `ready` 来源，并计入 `awaiting-acceptance`，口径需产品确认 | §12.1、§10.1 | `update_wiki_settings_core`；`pending_source_count` | 换路径 / 关闭开关的行为与文案一致；待分析只含尚未消费且可供编译的来源 |
| R8 | **部分** | 任务实时性与列表分页。引擎会发 `wiki://job-changed`，工作台未订阅，需手动刷新。`list_jobs` 返回数组，前端 `total === items.length`，超过一页看不到完整「加载更多」判断 | §12.3 | `engine.rs` `WIKI_JOB_CHANGED_EVENT`；`wiki-jobs-view.tsx`；`wiki-types.ts` `normalizeWikiList` | 状态变化自动刷新；分页总数来自服务端 |
| R9 | **未做** | 桌面打开入口。「用 Obsidian 打开」为禁用占位；无「在系统中显示」 | §12.2 | `wiki-page.tsx` `OpenInObsidianButton` | 仅本地桌面可用；Web / 远程不假装打开服务器路径 |
| R10 | **部分** | 导入后的整理材料。提取成功时 import job 直接标 `succeeded`，不保证走过 ingest 摘要。资料页 `importMeta()` 只传标题 / URL / 作者 / 材料角色 / 本人角色，不传 `project_ids` / `area_ids`（API 已预留） | §8.1、§9.1 | `wiki/import.rs` `job_status`；`wiki-sources-view.tsx` `importMeta`；`src/lib/api.ts` | 导入与 ACP 走同一 ingest→compile 链；用户能补充项目 / 职责归属 |
| R11 | **未做** | ACP 入队非持久。`dispatch_run_settled` 调 `enqueue_persist`，内部 `tokio::spawn`。进程在首次来源落库前崩溃会丢这一轮；入队失败没有方案要求的受限重试与任务页可见记录（现有失败路径会尝试插一条 `persist_failed` job，成功跳过只打日志） | §7.2、§10.2 | `acp/run_settled.rs`；`wiki/source.rs` `enqueue_persist` | 入队失败在任务页可见并可重试；不把内存队列写成 durable outbox |

## 5. 明确后置（本期不要做）

与方案 §1.1「后续再做」一致，实现时也不要顺手做：

- Codeg 内图谱、Canvas、向量检索、RAG
- OCR、扫描件、PPTX / XLSX、音视频
- 磁盘监听、账号连接器、多租户或多 vault
- 用会话次数或模型置信度生成能力等级

## 6. 建议下一轮顺序

1. **R1 + R2**：项目绑定和提交前证据校验，避免错误知识沉淀。
2. **R3**：撤回 / 原件 / 冲突，可对外试用的底线。
3. **R4**：工作 / 能力产品视图，用户能按合同阅读而不只是翻文件。
4. **R5–R8、R11**：Worker、取消、设置、任务页、入队可见性。
5. **R9–R10**：桌面打开与导入归属。

## 7. 主要代码落点

`src-tauri/src/wiki/`、`src-tauri/src/document_extract/`、DB entities / migrations / `wiki_service`、`commands/wiki.rs`、`commands/wiki_engine.rs`、Web handlers / router、桌面与 server 启动点、`agent-skills/wiki-*`、`src/app/settings/wiki/`、`src/components/wiki/`、workbench route、`src/lib/{api,wiki-types}.ts`、i18n。
