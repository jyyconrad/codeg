# 个人 Wiki 整理 Agent 提升方案

| 字段 | 值 |
| --- | --- |
| 日期 | 2026-09-18 |
| 对象 | WikiWorker 三层任务：`turn_summary` / `session_rollup` / `wiki_synthesize` |
| 对照 | [内容问题记录](./wiki-content-quality-issues.md)、[数据修复方案](./wiki-content-data-improvement.md)、[记忆流水线](./wiki-memory-pipeline-design.md) |
| 范围 | **运行机制、宿主包装/校验、SKILL、内置 prompt、JSON 合同**。不重写已落盘笔记（那是数据方案）；不恢复 WeKnora 候选编译 |

## 1. 要解决什么

单轮摘要已经能写人能读的「这一轮做了什么」。整理 Agent 的失败在于**没有把记忆编译进会变厚的 Wiki**：

- 有项目绑定的 turn 被消费后，可以只留下一篇同名工作记录，项目页仍为空。
- 宿主把证据和来源包装成固定模板，模型即使写对了也会被改错或写重。
- 入口页、资料 index、log 由代码生成，模型维护不了知识地图。
- 对话不点「完成」就没有 session，归纳永远对着 UUID 叶子。

目标：一次成功的归纳之后，绑定了项目的记忆必须落到**可更新的权威页**（项目 / 决策 / 方法 / 实体），而不是又一篇平行摘要。单轮页继续存在，但不再是工作 Wiki 的主入口。

## 2. 问题如何落到代码

| 问题 | 运行时 | Skill / prompt | 宿主包装与校验 |
| --- | --- | --- | --- |
| P1 无项目页 | `batching.rs` 按 `rel`（UUID）切，每批最多 8 条、6 万字，**不按项目分组**；`list_pending_inputs` 一旦关联就标记消费 | skill 提到 `work/projects/`，但未要求「有 binding 则必须 update/create 项目页」 | 合同示例 `type: method` + `op: create`，引导建方法页 |
| P2 入口是目录 | `library.rs` 每次刷新用子目录链接覆盖 `index.md` / `work/index.md` | 模型即使写地图也无法提交为稳定入口 | `DOMAIN_TYPES` 不含 `index` |
| P3 无会话页 | `enqueue_on_completed` 只在 `ConversationStatus::Completed` | session skill 本身可归并 | 桌面 ACP 长期 InProgress 则任务永不入队 |
| P4 复印件 | 每批只看见本批 turn；索引过滤掉 turn/session（`index_payload` 只要 `DOMAIN_TYPES`） | 要求合并，但「必要时新建」无门槛 | 不拒绝与已有标题近似的 `create` |
| P5 过时并存 | turn 任务不能写 decision/project | 未强制对同主题旧结论 `supersede` | `supersede` 可用但示例不出现 |
| P6 错类 | 无类型检查 | 方法/成果/记录边界写得软 | 接受任意 `DOMAIN_TYPES` |
| P7 证据空转 | — | 禁止从代理执行升格**个人熟练度**（合理）但未提供「工作任务页」 | `proposals.rs` 写死 `knowledge_only` / `source_reported` / `unspecified` |
| P8 项目 UUID | — | — | `decorate_sources` 把 binding id 写入 `projects` |
| P9 无 sources 页 | 采集只写 `raw/`；library 用 `raw_path` + `source_title` 编资料目录 | 归纳不能提案 `source` | `DOMAIN_TYPES` 无 `source`；turn 包装混用过 `[[sources/id]]` 与不写 sources |
| P10 双来源 | — | 模型常在 body 写 `## 来源` | `proposals.rs` 再追加一节 |
| P11 跟文件夹走 | 绑定来自会话根目录 | 未要求把私事与工程拆到 area | 项目元数据是 git 快照，当「项目背景」喂给模型 |
| P12 无 log | 仅 `import.rs` 调 `append_log_idempotent` | — | turn/synthesize 成功不写 `log.md`；journal 不在可写类型里 |
| 读项目根 | 设计允许附加只读项目目录 | skill 说按需读 | `compile.rs` payload **没有**项目根路径；`extra_read_roots` 只加 staging 与来源父目录 |

当前轮次预算：turn 8、session 16、synthesize 24（`WIKI_TURN_SUMMARY_MAX_TURNS` / `WIKI_SESSION_ROLLUP_MAX_TURNS` / `WIKI_COMPILE_MAX_TURNS`）。预算不是主因；分批和消费语义才是。

## 3. 分层改什么

### 3.1 运行机制（先改，否则 prompt 再严也会被分批拆碎）

**按项目分批。** `plan_batches` 先按 `project_binding_ids`（无绑定的进「未分组」）分组，组内再受 8 条 / 60k 字 / token 预算限制。同一项目的 turn 不要和另一项目的 UUID 邻居混在一批。

**消费与权威页挂钩。** 一批里只要出现非空 `project_binding_id`，本批提案必须包含对该项目页的 `create` 或 `update`（找不到已有 `type: project` 且 binding 对应页则 create）。否则该批**不算消费**这些记忆（`processed` 不得把它们标 `used`），任务可记 warning `missing_project_page`，允许重试。无绑定的记忆仍可只更新方法/记录。

**会话封口。** 保留 Completed 入队。另加一条不抢用户会话的回填：Wiki 启用时，对「已有 ≥2 篇 turn、无 session 页、最近一篇 turn 超过 48h 且对话非 InProgress 活跃编辑」的 conversation 入队 `session_rollup`。聊天通道若仍在 `end_turn` 时标 Completed，保持现行为（一轮既有 turn 又有 session）。

**来源页由宿主生成，不经模型。** ACP 冻结或导入成功后写 `sources/{source_id}.md`：标题、种类、时间、raw 链接、贡献笔记。模型不再负责造来源页。`DOMAIN_TYPES` 不必对日常归纳开放 `source`。

**log。** turn / session / synthesize 提交成功后，按 job id 幂等追加 `log.md`（已有 `raw::append_log_idempotent`）。一行写清 kind、页路径或「无内容」。

**项目根只读。** 按记忆流水线：payload 提供本批 binding 的 canonical 目录列表，`FsAccessPolicy::wiki_worker` 附加只读根。Agent 不读不算失败。当前 `compile.rs` 未传这些路径，属于实现缺口，应补上。

**library 入口。** `work/index.md` 维护区继续由代码生成，但生成内容改为：有项目笔记则列出项目标题 + 一句话 summary + 更新时间；没有项目笔记再退回子目录链接。不要把 `metadata/projects` 的 git 快照当工作入口。资料 index 优先 turn 标题 / 来源阅读页标题，禁止用用户第一句 prompt。

### 3.2 宿主包装与校验（改完即可消灭一批「内容病」）

落点主要在 `src-tauri/src/wiki/synthesis/proposals.rs`、`compile.rs` 的 `decorate_sources`、`turn_summary.rs` 的 `wrap_memory_page`。

1. **来源节去重。** 追加 `## 来源` 前若 body 已有该标题或已列出相同 `input_rels`，不再追加。这是 P10 的直接原因。
2. **`projects` 写 wikilink。** 若 vault 中已有该 binding 的项目页，写入 `'[[work/projects/…]]'`；否则写 binding id 并在 warning 中说明待建页。禁止把 git metadata 路径当成项目笔记。
3. **turn YAML 对齐。** `wrap_memory_page` 同时写 `codeg_project_binding_id`、`projects`（有项目页则 wikilink）、`sources`（指向真实存在的 `sources/{id}` 或 `raw/…`，禁止链到不存在文件）。现在的包装只写了 `source_ids`，vault 里旧页还混着 `[[sources/uuid]]`。
4. **证据字段。** 停止对所有归纳页写死 `knowledge_only`。默认仍是 `source_reported` + `personal_role: unspecified`；`evidence_level`：能力页以外用 `knowledge_only` 可以保留；能力页仅在正文出现本人角色且证据类型不是「参考资料」时才升到 `practice_reported`。模型不得写「精通」。这与方案里「不能伪造角色」一致，只是不要用写死三元组把能力 Wiki 永久锁死。
5. **近重复 create。** 同 type、标题规范化后相同或包含已有标题，拒绝 `create`，要求 `update` 已有 `note_id`。可先做精确/包含匹配，不做模糊 Embedding。
6. **类型轻量门闩。** 标题或正文明显是一次环境操作（清库、重建镜像、seed）时，拒绝 `outcome`；明显是话术/汇报且无步骤-输入-输出时，拒绝 `method`。误杀用 warning + 改类型重试，不要整批失败。
7. **合同示例。** `llm.rs` 里 synthesize 示例从 `type: method` + `op: create` 改为同时展示「更新项目页」和「新建方法页」两条；`op` 示例以 `update` 为首。

### 3.3 Skill 与 prompt

三份 SKILL 与 `prompts/*.md` 职责已经分开（skill 可被设置覆盖，task 模板由宿主拼合同）。改 skill 时同步改同名内置 prompt，避免设置「恢复默认」只恢复一半。

**wiki-synthesize（主改）**

- 有 `project_metadata` / binding 时，本批第一件事是定位或创建该项目权威页，再决定是否需要独立 record/decision。
- 默认 `update` 已有同主题页；只有新主题、且无法并入项目/方法时才 `create`。
- 发现同主题多篇 `active` 结论时，必须留下一篇当前有效，其余 `supersede` 或在权威页用时间线收口，禁止平行现行。
- 类型：`method` 必须有可迁移步骤与适用边界；一次联调/清库进 `work-record` 或不要建页；面试沟通进 `area` 或 record，不进 methods。
- `entity` 只给在本批和已有页中反复出现的系统/产品；不为每个文件名建页。
- 能力页写「可复用工作任务 + 实践记录 + 边界」，执行者写清是代理还是用户；没有本人角色就不要建能力页（保持空优于编造）。
- 读完本批 turn 后，用 `grep` 在 `work/projects`、`work/decisions` 查同一主题旧结论。

**wiki-turn-summary**

- 继续只写一篇 turn，不改项目页（保持分层）。
- 正文增加「建议归入的权威页」小节（项目名/决策主题），便于归纳检索；不是 wikilink 也行，避免 turn 任务去造不存在的项目路径。
- 禁止在 body 再写与宿主重复的 `## 来源` 清单（来源由包装层写 YAML + 单一节）。若保留「原始记录」链接，只用一条且目标必须是任务信息里的 raw 路径。

**wiki-session-rollup**

- 明确：输出是对话的**最终状态**，中间被推翻的方案只保留为「曾考虑」，不要按 turn 列表复述。
- `turns` wikilink 由宿主写 YAML（现已有 `turn_rels`），模型不要在正文再贴一遍 UUID 列表。

**AGENTS.md**

- 初始化模板从三行占位改为可编辑的组织偏好提纲（活跃项目、私事与工程分离、权威页规则）。仍禁止模型改此文件（`fs_policy` 已拒写）。数据方案负责填本机内容。

## 4. 分阶段交付

### 阶段 A — 宿主缺陷（不改模型行为也能修好 P8/P10/P9 的一半）

- 来源节去重
- `projects` wikilink
- turn YAML `sources` 指向真实文件
- 采集后写 `sources/{id}.md`
- 成功任务写 `log.md`
- 资料 index 标题策略

验收：新归纳页不再出现两个 `## 来源`；新 turn 不再出现悬空 `[[sources/uuid]]`；`log.md` 在新 turn 后有记录。

### 阶段 B — 分批、消费、合同示例（P1/P4 主因）

- 按项目分批
- 有 binding 必须提案项目页，否则不消费
- 近标题拒绝 create
- 合同示例改为 update 项目页

验收：对仅有 switchgear binding 的一批测试记忆，提交后 `work/projects/` 出现或更新一页；不会只新增一篇与 turn 同名的 record 并把记忆标已消费。

### 阶段 C — skill/prompt 与类型门闩（P5/P6/P7）

- 三份 skill + 内置 prompt
- outcome/method 轻量拒绝
- 证据字段不再全库写死（能力页规则见 3.2.4）

验收：用「面试话术」和「清库 seed」两份夹具 turn 跑归纳，不得写入 `knowledge/methods` / `outcome`；用「向导方案被后一轮推翻」的两份 turn，后一轮归纳后只剩一条当前有效决策。

### 阶段 D — 会话封口与工作入口（P2/P3）

- 闲置多轮回填 session
- `work/index.md` 列出项目笔记而非空目录说明
- 项目根只读路径进入 synthesize payload

验收：未点完成、但 48h 无新 turn 的多轮对话出现 session 页；工作首页能看到项目标题。

阶段 A 可与数据方案 4.2 并行。阶段 B 完成前，不要用现网 synthesize 做大规模项目回填。

## 5. 明确不做

- 不把 turn 任务升级成直接改项目页（破坏「先记忆后归纳」）。
- 不引入向量检索、图谱、自动能力打分。
- 不让模型写 `AGENTS.md`、`raw/`、metadata git 快照。
- 不把导入文档自动拉进 `wiki_synthesize`（产品已锁定）；来源阅读页由宿主生成即可。
- 不把 library 改成模型自由撰写的首页（刷新冲突）；只改生成规则。

## 6. 测试要点

现有 `cargo test --lib wiki::` 继续作回归。至少补：

| 测试 | 期望 |
| --- | --- |
| `validate_output` 在 body 已有 `## 来源` 时不重复追加 | 正文仅一节 |
| `decorate_sources` 在存在项目页时写入 wikilink | YAML 不含裸 UUID 或同时含可解析链接 |
| `plan_batches` 两项目各 5 条 turn | 不会出现跨项目同一批（除非未分组溢出策略有文档） |
| 有 binding 的批次只返回 record、无 project | 记忆不进入 consumed |
| 与已有 method 同标题的 create | `invalid_output` |
| 采集后 `sources/{id}.md` 存在 | library 资料链可打开 |
| Completed 仍入队 session；InProgress + 新 turn 不入队闲置回填 | 不误封口 |

不把「文笔好」当通过条件。通过条件是：权威页在、消费语义对、链接存在、模板不重复。

## 7. 与数据方案的接口

| 数据方案步骤 | 依赖本方案 |
| --- | --- |
| 4.2 机械断链、去重来源 | 不依赖；阶段 A 防止再污染 |
| 4.3 会话回填 | 阶段 D 的闲置入队可替代部分人工名单 |
| 4.4 项目权威页 | **必须等阶段 B**，否则会再写平行 record |
| 4.5 错类挪页 | 阶段 C 防止再写入错误 type |
| 4.6 AGENTS.md | 不依赖代码；阶段 C 后模型会读它 |

两边都做完的验收：switchgear 项目页能回答当前向导口径；工作首页能列出该项目；新一轮相关 turn 之后是**更新**该页，而不是再多一篇同名工作记录。
