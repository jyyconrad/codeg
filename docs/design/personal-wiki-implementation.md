# 个人 Wiki 实现进度与待完善

| 字段 | 值 |
| --- | --- |
| 文档标题 | 个人 Wiki 实现进度与待完善 |
| 日期 | 2026-09-13 |
| 状态 | 主干可试用，**未达到方案 v1 验收** |
| 分支 / 提交 | `develop-20260909` / 本次修复提交（见 git log） |
| 对照基线 | `develop-20260909` |
| 方案合同 | 本地 `docs/superpowers/specs/2026-09-12-personal-wiki-design.md`（该目录在 `.gitignore`，不入库） |
| 事实口径 | **目标**以方案 §1–14 为准。**实现**以本分支代码为准。未完成项不得写成已交付 |

本文只记当前代码做到哪、还差什么。页面合同、采集规则、Worker 边界仍以方案正文为准，不在这里改写。

未完成撤回/冲突、Worker 工具沙箱、实时任务刷新和桌面打开入口前，发布说明仍应使用「可试用的知识采集与阅读主干」口径。

## 1. 当前结论

打开 **设置 → 个人 Wiki**（默认关闭）后，ACP 成功轮次可以冻结入库，外部 Markdown / TXT / 文本 PDF / DOCX 可以导入，工作台可以浏览 vault 里的 Markdown，任务页可以立即整理、重试、取消。

还不能按方案验收为「个人工作 Wiki / 个人能力 Wiki」：

- 项目绑定已接通，ACP 来源会按根目录关联稳定项目标识。
- 能力证据已按来源标注校验，模型不能伪造个人角色或核验状态。
- 多文件导入、批次结果和统一 ingest 队列已接通。
- 仍缺撤回、原件下载、冲突处理、LSP MCP 门面和 Obsidian 桌面入口。

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
| 证据压级 | 来源标注按 source 校验，模型不能伪造个人角色或核验状态 | `sanitize_capability_evidence` |
| 代码智能 ACP 注入 | 支持 MCP 的外部 ACP 会话可获得官方 CodeGraph MCP；缺失时失败开放 | `acp/connection.rs` |
| 项目与批次 | ACP 来源建立项目 binding；多文件导入返回每文件 source/job/status | `wiki_service.rs`、`wiki/import.rs` |
| 默认路径 | vault：`$CODEG_HOME/wiki` → `$CODEG_DATA_DIR/wiki` → `~/.codeg/wiki`；原件在 vault 外 `wiki-state/originals/` | `wiki/paths.rs` |

单测口径（记录时）：在 `src-tauri/` 执行

```bash
cargo test --no-default-features --features test-utils --lib wiki
```

历史记录为 86 项；本次新增了项目绑定、证据、批次导入、取消和恢复测试。由于当前 Rust 工具链版本限制，本次未能重新执行 Rust 测试；这些测试不代替方案 §14 用户场景和桌面 / 服务器全量门禁。

## 3. 分阶段切片现状

相对 `develop-20260909` 当前修复工作区，不是对方案合同的改写。

| 切片 | 方案目标 | 现状 |
| --- | --- | --- |
| PR1 来源基础与 ACP 采集 | 同锁冻结、稳定 `run_id`、vault/source/job、过滤、确定性 raw | **已合入。** pending 快照原子落盘并可启动恢复；durable outbox 仍可继续增强 |
| PR2 外部导入 | 受控上传 / 粘贴、四种格式、哈希版本、原件、覆盖预览 | **已合入。** 多文件批次和部分失败可见；撤回、纠错提取、原件下载、版本 UI 未做 |
| PR3 Worker 与提交 | 工具沙箱、暂存提案、校验提交、分段、恢复 | **部分合入。** 取消/超时已有运行时信号；生产编译仍以 JSON 调用为主，完整工具沙箱和长文归并未完成 |
| PR4 工作与能力编译 | 项目映射、四步编译、证据规则、入口与日记、cron | **部分合入。** 项目 binding、证据校验和工作/能力卡片已接通；完整案例聚合仍待补齐 |
| PR5 设置与工作台 | 四视图、导入标注、冲突、Obsidian 入口、i18n | **部分合入。** 批次导入和项目/证据浏览可用；冲突、撤回、实时刷新、桌面打开未做 |

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

**阻塞产品表述**（R3、R5–R9 未完成时，不要对外宣称双 Wiki 已完成 v1）：

| 编号 | 状态 | 缺口 | 方案 | 代码线索 | 建议验收 |
| --- | --- | --- | --- | --- | --- |
| R1 | **已完成** | ACP 来源按数据库实例、vault 和根目录建立稳定 binding，并写入 source project_ids；项目卡片可读取 binding | §4.1、§10.1 | `db/service/wiki_service.rs`、`commands/wiki.rs`、`wiki/source.rs` | 同一根目录复用 binding；不同根目录不合并 |
| R2 | **已完成** | 能力 proposal 的 personal/material role 只取 DB 标注；正文角色和核验状态会被宿主校正；混合来源按来源逐条降级 | §4.2、§9.3、§11.3 | `wiki/compile.rs` `sanitize_capability_evidence` | 参考资料和模型伪造字段不能升级个人实践证据 |
| R3 | **未做** | 纠正与隐私闭环未做。无撤回来源、无按 `source_id` 下载原件、无冲突查看 / 解决 API 与界面。冲突文件只落到 `wiki-state/conflicts/`。`wiki_reextract` 已有，纠错提取（新 raw）未做 | §12.3、§13、§11.3.5 | `commands/wiki.rs` 无 withdraw / original download / conflict；`wiki-page.tsx` 无对应操作 | 撤回后旧标注不再支撑新结论；用户能下载原件核对；部分提交冲突时暂停该 vault 后续写入直至处理 |
| R4 | **部分** | 工作页已有项目卡片，能力页已有 own-work 证据列表；完整案例聚合、边界和下一次实践仍未展示 | §12.2、§1 | `wiki-work-view.tsx`、`wiki-capabilities-view.tsx` | 项目/能力按类型和证据浏览，并能追到具体 note/source |

**运行可靠性与体验**（不补齐则试用不稳定，或与设置文案不符）：

| 编号 | 状态 | 缺口 | 方案 | 代码线索 | 建议验收 |
| --- | --- | --- | --- | --- | --- |
| R5 | **部分** | Worker 与方案中的工具沙箱不一致。`NativeTurnTools::wiki_compile` 已定义且有单测，生产编译基本走无工具 JSON 调用；长文候选未按批次归并 | §9.2、§11.1–11.2 | `agent/model/turn.rs` `wiki_compile`；生产路径在 `wiki/compile.rs` / `wiki/llm.rs` | 编译只读清单内笔记与 raw 分段，只写当前 job 暂存；超长来源分段覆盖可见且不静默丢段 |
| R6 | **部分** | 取消/超时已能唤醒 Worker 并阻止成功状态回写；提交函数内部仍缺独立 DB 终态 CAS，极窄竞态需下一轮补齐 | §10.2 | `wiki/engine.rs`、`commands/wiki_engine.rs` | 取消或超时后不再提交文件；可重试错误按退避排队 |
| R7 | **部分** | vault/state root 已统一按设置动态解析；关闭总开关和切换路径的完整排空/用户提示仍待补齐 | §12.1、§10.1 | `wiki/engine.rs`、`commands/wiki.rs` | 换路径 / 关闭开关的行为与文案一致 |
| R8 | **部分** | 任务实时性与列表分页。引擎会发 `wiki://job-changed`，工作台未订阅，需手动刷新。`list_jobs` 返回数组，前端 `total === items.length`，超过一页看不到完整「加载更多」判断 | §12.3 | `engine.rs` `WIKI_JOB_CHANGED_EVENT`；`wiki-jobs-view.tsx`；`wiki-types.ts` `normalizeWikiList` | 状态变化自动刷新；分页总数来自服务端 |
| R9 | **未做** | 桌面打开入口。「用 Obsidian 打开」为禁用占位；无「在系统中显示」 | §12.2 | `wiki-page.tsx` `OpenInObsidianButton` | 仅本地桌面可用；Web / 远程不假装打开服务器路径 |
| R10 | **已完成** | 导入统一进入 ingest job；多文件批次逐项返回 source/job/status，项目与职责字段保留 | §8.1、§9.1 | `wiki/import.rs`、`wiki-sources-view.tsx`、`src/lib/api.ts` | 导入与 ACP 走同一 ingest→compile 链；用户能补充项目 / 职责归属 |
| R11 | **已完成** | ACP 快照在异步入队前原子写入 pending-acp，启动恢复会重放且按 run_id 幂等；失败文件保留待重试 | §7.2、§10.2 | `acp/run_settled.rs`、`wiki/source.rs`、`wiki/engine.rs` | 进程崩溃后快照可恢复；入队失败可见且不丢失 |

## 5. 明确后置（本期不要做）

与方案 §1.1「后续再做」一致，实现时也不要顺手做：

- Codeg 内图谱、Canvas、向量检索、RAG
- OCR、扫描件、PPTX / XLSX、音视频
- 磁盘监听、账号连接器、多租户或多 vault
- 用会话次数或模型置信度生成能力等级

## 6. 建议下一轮顺序

1. **R3**：撤回、原件下载、纠错提取和冲突查看/解决，形成隐私与纠正闭环。
2. **R5–R8**：切换到受限工具沙箱，补长文归并、取消提交 CAS、设置排空和任务实时刷新。
3. **R4**：补齐能力案例聚合、边界和下一次实践；工作页追踪项目 note 而不只显示来源。
4. **R9**：实现桌面“在系统中显示/用 Obsidian 打开”，远程/Web 继续只提供受控浏览。
5. **LSP MCP**：设计 LspMcpRequest、workspace pool lease、broker 取消链路后，再向外部 ACP 注入统一 LSP 门面。

## 7. 主要代码落点

`src-tauri/src/wiki/`、`src-tauri/src/document_extract/`、DB entities / migrations / `wiki_service`、`commands/wiki.rs`、`commands/wiki_engine.rs`、Web handlers / router、桌面与 server 启动点、`agent-skills/wiki-*`、`src/app/settings/wiki/`、`src/components/wiki/`、workbench route、`src/lib/{api,wiki-types}.ts`、i18n。
