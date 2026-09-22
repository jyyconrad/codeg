# 个人 Wiki 内容问题记录（LLM-Wiki 视角）

| 字段 | 值 |
| --- | --- |
| 日期 | 2026-09-18 |
| 对象 | 本机个人 Wiki vault：`/Users/jiangyayun/.codeg/wiki` |
| 视角 | LLM-Wiki（Karpathy：raw 只读、Wiki 由模型增量编译、Schema 约束结构；对照本仓库 [个人工作 Wiki 与个人能力 Wiki 方案](../superpowers/specs/2026-09-12-personal-wiki-design.md)） |
| 范围 | **已生成笔记的内容、结构和可导航性**。不审界面交互，不把“流水线尚未实现”单独写成代码缺陷，只记录它们在 vault 里留下的内容后果 |
| 快照口径 | 2026-09-18 对 vault 全量 `.md` 计数与抽样阅读；数字只描述当时文件，不是产品统计口径 |
| 后续方案 | [数据修复](./wiki-content-data-improvement.md)；[整理 Agent](./wiki-organize-agent-improvement.md) |

## 1. 用什么标准看

LLM-Wiki 要的不是“每轮对话存一份摘要”，而是一份**可复利的知识库**：

1. 原始材料进 `raw/`，保持只读。
2. 模型把材料**编译进已有页面**：项目、职责、实体、概念、方法、能力；新资料应更新旧页，而不是另开一篇互不相关的叶子。
3. `index.md` / 入口页是给人和 LLM 用的**知识地图**，能回答现在在做什么、当前结论是什么、缺口在哪。
4. 页面之间用稳定 wikilink 交叉引用；矛盾、过时结论、断裂链接应能被巡检出来。
5. 本产品额外要求：工作 Wiki 能回答项目进展与决策；能力 Wiki 能回答可复用方法与证据边界。会话摘要只是中间材料。

对照这条标准，当前 vault 的主要形态是：**单轮记忆很多，编译层几乎空，入口页只是目录。**

## 2. 当时库里有什么

| 层级 | 目录 | 非 index 页 | 对照 LLM-Wiki / 产品合同 |
| --- | --- | --- | --- |
| 原始材料 | `raw/sessions/` | 45 | 有冻结会话 |
| 原始材料 | `raw/imports/` | 11 | 有导入文档，未再编译成来源摘要页 |
| 记忆（中间层） | `work/turns/` | **121** | 单轮摘要是主存量 |
| 记忆（中间层） | `work/sessions/` | **0** | 对话完成后应有会话页 |
| 工作编译 | `work/projects/` | **0** | 项目页应汇总目标、进展、决策 |
| 工作编译 | `work/areas/` | **0** | 职责页空 |
| 工作编译 | `work/records/` | 20 | 多为单轮改写，很少跨轮合并 |
| 工作编译 | `work/decisions/` | 4 | 有少量决策，未挂到项目页 |
| 工作编译 | `work/outcomes/` | 2 | 数量与 121 轮工作不匹配 |
| 能力编译 | `capabilities/` | **0** | 能力 Wiki 无正文 |
| 共享知识 | `knowledge/methods/` | 20 | 有方法，但类型混杂 |
| 共享知识 | `knowledge/concepts/` | 2 | 过薄 |
| 共享知识 | `knowledge/entities/` | **0** | 开关柜、网关、Rig、iFlyCode 等未建实体 |
| 来源阅读页 | `sources/` | **0**（仅 index） | 合同要求每份来源有可读摘要页 |
| 日志 / 日汇总 | `log.md` / `journal/` | 仅 9-14 导入成功；journal 空 | 不是知识变更日志 |
| Schema | `AGENTS.md` | 3 行占位 | 没有领域结构、命名、合并规则 |
| 代码元数据 | `metadata/projects/` | 15 | Git/目录快照，不是项目知识页 |

绑定了项目的单轮记录：switchgear 58、codeg 33、iflycode-agent 11、简历 8、xingye 6。这些数量足够写项目页，但 `work/projects/` 仍为空。

## 3. 问题清单

### P1 知识停在摘要，没有编译成 Wiki

LLM-Wiki 的核心动作是 ingest 后**更新实体/概念/项目页**。当前 121 篇 `turn-summary` 是主入口；项目、职责、能力、实体、对话总结均为空。

结果：人和模型都无法从 Wiki 直接得到“switchgear 现在做到哪、当前生效决策是什么”，只能在 58 篇单轮记录里重读。这正是 RAG 式“每次从碎片拼答案”，不是 Wiki 复利。

例：对话 783/784 连续讨论设备设置向导（单串口 → 多串口 → 保存后手动重启 → 进程内重载），每轮各写一篇 turn，没有一篇项目页或决策页收口为**当前有效方案**。

### P2 入口页是目录，不是知识地图

`index.md`、`work/index.md` 只有子目录链接。工作首页不能列出：进行中的项目、未关闭问题、最近结论、证据缺口。

界面上打开「工作」时，正文是索引列表，资料区显示「还没有资料」（索引页本身没有来源）。对 LLM 导航同样无效：读首页得不到任何事实。

`overview.md` 不存在。`AGENTS.md` 没有告诉维护智能体本库的主题边界和应维护的权威页。

### P3 对话未封口，多轮工作没有会话页

同一 `conversation_id` 下常见 5–11 轮（784 有 11 轮、783 有 10 轮、543 有 7 轮），`work/sessions/` 仍为「此目录暂时没有笔记」。

产品合同里 session 页负责「这次对话做了什么」。缺失后，synthesize 只能面对一堆叶子摘要，更容易做成“再摘要一遍单轮”，而不是主题归并。

### P4 归纳页大多是单轮复印件，不是跨轮合并

`work/records/` 里多篇标题与某篇 turn 几乎相同，YAML `sources` 常只有 1 个 turn id。例如：

- 《诊断报告抽屉与实时诊断输入链路校准方案》← 单篇 turn
- 《Agent 运行时架构验收差距与修复优先级》← 同名 turn
- 《开发环境清库后重新运行 mock seed》← 单次联调操作

LLM-Wiki 要求后续轮次**修正旧页**。这里是平行再写一篇，历史中间态仍以 `status: active` 并列。

### P5 过时结论没有权威页，互相打架

设备向导相关 turn 同时保留：

- 仅支持单串口、需评估多选
- 改为全新部署并统一多串口
- 首版收敛为保存配置、手动重启生效
- 进程内重载、`applySetup` 串行化（材料写明尚未实现）

没有 `decision` 或 `project` 页声明哪一条是当前有效、哪一条已 superseded。Lint 视角下这是未处理矛盾。

同类：总览气体诊断从「类型与时间拼成一行」到「时间改小字、去掉未更新标签」，成果页只吸了后面一轮，前面 turn 仍像当前事实。

### P6 页面类型用错，方法/成果被一次性事件占满

| 页面 | 问题 |
| --- | --- |
| `knowledge/methods/岗位汇报中表达岗位匹配与业务兴趣-*.md` | 面试微信话术，不是可迁移工程方法 |
| `work/outcomes/docker-开发环境前端镜像重建记录-*.md` | 一次环境操作，不是可复用成果 |
| `work/records/开发环境清库后重新运行-mock-seed-*.md` | 一次清库 seed，应进 journal 或操作记录，不宜当长期工作事实 |
| `knowledge/methods/codeg-windows-安装包-*-nsis-*.md` | 内容接近操作手册，作为 method 尚可，但未链到 codeg 项目页（项目页也不存在） |

概念只有 2 篇；大量本该是实体（switchgear、gas-monitor-device、边缘网关、Rig、iFlyCode）的对象没有实体页。

### P7 能力 Wiki 全空，证据字段空转

121 轮工作 + 20 篇方法之后，`capabilities/` 仍是「此目录暂时没有笔记」。

已编译页几乎统一：

- `evidence_level: knowledge_only`
- `verification_status: source_reported`
- `personal_role: unspecified`

无法回答产品问题：“我有哪些实践证据、边界和下一次练习”。代理完成的实现（总览改版、NSIS 口径、seed 成功）也没有升格为带角色的 application/result 证据，只是把整库锁在“来源声称”。

### P8 项目身份对不上，UUID 不能当链接

编译页 YAML 形如：

```yaml
projects:
- 77894a80-b65b-475a-bbef-857b81ffb995
```

这不是 wikilink，也不能跳到 `work/projects/`（该目录无笔记）。真正叫 “switchgear” 的文件在 `metadata/projects/`，内容是采集时刻的 git 分支/HEAD，没有目标、进展、决策。

目录里还有「项目 15」「项目 16」「develop」「code」这类文件夹名，进入「项目与文件夹」后对人和 LLM 都是噪声，不能当工作 Wiki 的项目入口。

单轮页则用 `codeg_project_binding_id`，没有 `projects:` wikilink，所以 121 篇 turn 在 YAML 层全部“无项目列表”。

### P9 来源层断裂：sources 页不存在，链接指向 raw 或空路径

- `sources/` 只有 index，index 直接链到 `raw/sessions/`、`raw/imports/`。
- 至少 12 篇 turn 的 YAML `sources` 指向 `[[sources/<uuid>]]`，对应文件不存在。
- 其余部分 turn 链到 `raw/sessions/<uuid>`，跳过了合同中的来源阅读页。
- 编译页「查看资料」对索引页为空，与 45+11 份 raw 并存，阅读路径不闭合。

来源 index 标题大量是会话首句截断，例如「不需要做复杂的输入冻结和信息验证。前期只标记…」，不能当资料名。

11 份 `raw/imports/`（含方案 C 诊断窗、网关阈值、开关柜架构手册等）停在 document-dump，没有 source summary，也没有进入项目页。按当前产品规则导入不自动归纳，但 LLM-Wiki 仍要求至少有来源摘要页；现在连摘要页都没有。

### P10 交叉引用弱，模板把「来源」写了两遍

抽样中，绝大多数 turn 正文不链到方法、概念、决策或项目；只有文末「原始记录」。知识图是星形指向 raw，不是网状 Wiki。

归纳页普遍连续两个 `## 来源` 小节，内容重复（一篇指向可读标题，一篇指向路径）。至少 records / outcomes / methods 中 20+ 篇如此，例如：

- `work/outcomes/总览页气体诊断与测点卡片布局调整-fde22e85ebcf.md`
- `work/records/诊断报告抽屉与实时诊断输入链路校准方案-447901bdb67c.md`
- `knowledge/methods/岗位汇报中表达岗位匹配与业务兴趣-c9458e1393e4.md`

这是生成模板问题，直接降低可读性。

### P11 会话主题被绑到错误项目，或同一对话混进无关工作

- 面试汇报 turn 绑在项目「简历」上，方法页也挂同一 UUID；没有“个人沟通/求职”职责页，工程 Wiki 与私事材料混在同一套工作索引里。
- `conversation_id=781` 同时出现「Xingye 项目源码交付包完成打包」和「新诊断后旧优化任务解绑」，主题不该进同一会话记忆，更不该在没有项目页的情况下靠 binding 硬分。

### P12 日志不能支撑巡检

`log.md` 只有 2026-09-14 一批 `import succeeded`，之后大量 turn/synthesize 没有追加。`journal/` 无日汇总。

LLM-Wiki 的 log 应能回答：哪天 ingest 了什么、更新了哪些页。当前无法从 log 重建知识变更，只能扫文件树。

### P13 单轮正文质量不差，但停在“本轮做了什么”

抽样 turn（总览改版、向导方案、Wiki 架构清理）结构清楚：结果、限制、未验证项分开写，有的还标明“代理报告 / 快照截断”。这比早期“来源标题墙”可用。

问题不在单篇文笔，而在**没有下一跳**：不更新权威页、不标 superseded、不链实体、不收口会话。好摘要堆在一起，仍然不是 Wiki。

相对更好的反例：`knowledge/concepts/diagnosis-acceptance-time-boundaries-*.md` 有主张、边界、来源行号，接近 LLM-Wiki 概念页；但没有实体页和项目页接住它，也没有被后续 switchgear 工作回写。

## 4. 按 LLM-Wiki 三层对照

| 层 | 期望 | 现状 |
| --- | --- | --- |
| raw | 不可变原文 | 有 sessions/imports；标题差、未隐藏到高级入口（内容层表现为资料 index 被 raw 占满） |
| wiki | 编译后的项目/实体/概念/能力，增量更新 | 叶子摘要为主；编译目录空或薄；重复页多、权威页少 |
| schema | `AGENTS.md` 定义结构、类型、合并与巡检 | 几乎空白，维护智能体没有本库约定可遵守 |

三种核心操作里：ingest（turn）在产出叶子；query 只能扫摘要；**lint 从未把矛盾和缺页写回库内**。本文件补的是这一次 lint 记录。

## 5. 问题归属（数据 vs 整理 Agent）

同一内容后果往往两边都有痕迹。下表只定**主因**和**先修哪边**；详细改法见：

- [数据修复与提升方案](./wiki-content-data-improvement.md)
- [整理 Agent（运行时 / skill / prompt）提升方案](./wiki-organize-agent-improvement.md)

「数据」指 vault 里已经落盘的笔记、来源标题、对话状态、断链和错误分类。「Agent」包含 WikiWorker 领取与分批、宿主包装/校验、SKILL.md 与内置 prompt、JSON 交付合同。

| 编号 | 主因 | 数据侧留下的账 | Agent 侧对应机制 |
| --- | --- | --- | --- |
| P1 未编译成 Wiki | Agent | 58/33/11 轮已够写项目页，但 `work/projects/` 仍空 | 归纳按 UUID 每批最多 8 条；合同示例偏向 `method`；有项目绑定时仍可只建工作记录就算消费完毕 |
| P2 入口不是地图 | Agent（library） | 首页/工作页只有目录链接 | `library.rs` 刷新用子目录列表覆盖 index；`DOMAIN_TYPES` 不含 index，模型写了也会被冲掉 |
| P3 无会话页 | Agent 触发过窄 | 多轮对话没有 `work/sessions/c{id}.md` | `session_rollup` 只在对话 `Completed` 入队；桌面 ACP 长期进行中则永远不跑 |
| P4 单轮复印件 | 两边 | 平行 `work-record` 与 turn 同名 | skill 要求合并，但交付示例是 `create`；宿主不拦近标题新建 |
| P5 过时结论并存 | 两边 | 向导/总览多篇 `status: active` | turn 不能改决策页；归纳很少用 `supersede` |
| P6 类型用错 | Agent 分类 + 数据已脏 | 面试话术在 methods，清库在 records | 类型定义偏软；宿主不校验类型与内容是否匹配 |
| P7 能力空、证据空转 | Agent 宿主策略 | 能力目录空 | 宿主写死 `knowledge_only` / `source_reported` / `personal_role: unspecified`；skill 禁止从代理执行升格能力 |
| P8 项目 UUID | Agent 宿主 | YAML `projects` 是 binding id | `decorate_sources` 写入 UUID 而非项目笔记 wikilink；turn 只用 `codeg_project_binding_id` |
| P9 来源断裂 | Agent 宿主 | 无 `sources/*.md`；12+ 断链；资料名是 prompt 截断 | 归纳可写类型不含 `source`；library 用来源表标题 + `raw_path` 编资料 index |
| P10 双「来源」、弱链接 | Agent 宿主 | 20+ 篇两个 `## 来源` | `synthesis/proposals.rs` 无条件再追加一节；模型正文里已经写过 |
| P11 错绑/混主题 | 数据为主 | 面试绑「简历」；同一 conversation 混两个项目 | 绑定跟文件夹走；模型未改落到职责页 |
| P12 日志空 | Agent 宿主 | `log.md` 只有 9-14 导入 | 只有 import 追加 log；turn/synthesize 不写；journal 不在归纳类型里 |
| P13 单轮正文尚可 | — | turn 多数可读，可当回填原料 | 缺的是强制下一跳（更新权威页），不是再把摘要写长 |

先修 Agent 才能阻止同类脏数据再生；已落盘的断链、错类、缺页仍要单独做一次数据修复，不能指望下次归纳自动收干净。

## 6. 建议的内容修复优先级（只针对 Wiki 正文，不展开实现）

1. 为实际有存量的项目各写一篇权威项目页：至少 switchgear、codeg、iflycode-agent；写当前目标、生效决策、未决问题，并链到已有 records/methods。
2. 把设备向导、总览诊断、传感器联调等系列 turn 收成决策/记录，旧中间态标 `superseded` 或合并进同一页。
3. 补会话页：至少对仍能对应 `conversation_id` 的已完成对话做 rollup。
4. 建来源阅读页，修掉指向不存在 `sources/<uuid>` 的链接；资料 index 不要用 prompt 截断当标题。
5. 面试话术、一次性清库/重建镜像从 methods/outcomes 挪走或降级，避免污染可复用知识。
6. 填 `AGENTS.md`：本库主题（工作项目 vs 个人事务）、权威页规则、禁止为每轮新建平行记录。
7. 有实践证据后再写能力页；不要为填目录编造熟练度。空能力可以保持空，但不应在 121 轮实现类工作后仍完全没有“可复用任务”页。

条目 1–5、7 的落盘修法见数据方案；要阻止再生成同类页见 Agent 方案。不要在 Agent 分批/消费语义改完前，用现网「立即整理」做大规模项目回填。

## 7. 抽样文件（便于复核）

- 空编译层：`work/projects/index.md`、`work/areas/index.md`、`work/sessions/index.md`、`capabilities/index.md`、`knowledge/entities/index.md`、`sources/index.md`
- 入口无事实：`index.md`、`work/index.md`、`AGENTS.md`、`log.md`
- 元数据冒充项目：`metadata/projects/77894a80-b65b-475a-bbef-857b81ffb995.md`
- 过时并存：`work/turns/421b7042-d096-46f5-b81d-38a98ece64a1.md`（单串口）、`work/turns/fcc6def7-4f1b-4211-83f4-441857a47f6d.md`（保存后手动重启）
- 单轮复制成 record：`work/records/诊断报告抽屉与实时诊断输入链路校准方案-447901bdb67c.md`
- 双「来源」：`work/outcomes/总览页气体诊断与测点卡片布局调整-fde22e85ebcf.md`
- 类型错放：`knowledge/methods/岗位汇报中表达岗位匹配与业务兴趣-c9458e1393e4.md`
- 断链：`work/turns/4b165161-a9de-46cb-a84f-28a9ea3d7289.md` → `[[sources/4b165161-a9de-46cb-a84f-28a9ea3d7289]]`
- 接近合格的概念页：`knowledge/concepts/diagnosis-acceptance-time-boundaries-83ea9af5f831.md`
