# FDE 如何在 B 端做 AI 落地：作业模型 × Ontology Lite × 工作台/Agent × Demo 框架

| 字段 | 内容 |
|------|------|
| **日期** | 2026-10-15（Asia/Manila / UTC+8） |
| **状态** | 正式方案（独立轨道；可供运营/工程拆里程碑；**未声称已实现**） |
| **读者** | Mack、产品、工程、FDE/交付从业者 |
| **性质** | **新正式方案轨道**：FDE 运营层 + Ontology Lite + Demo/基线框架 |
| **调研输入** | (1) [`../../research/fde-ontology-b2b-ai-2026-10-15/`](../../research/fde-ontology-b2b-ai-2026-10-15/) — FDE 美中矩阵与浅层 Ontology；(2) [`../../research/ontology-enterprise-agent-deep-2026-10-15/`](../../research/ontology-enterprise-agent-deep-2026-10-15/) — Ontology **深潜**；(3) [`../../research/fde-role-and-ai-tools-deep-2026-10-15/`](../../research/fde-role-and-ai-tools-deep-2026-10-15/) — FDE **角色+工具栈**加深；(4) [`../../research/fde-definition-value-deep-2026-10-15/`](../../research/fde-definition-value-deep-2026-10-15/) — FDE **定义/价值/度量三角**专管 **§2–§3** 加厚。只读引用；星数/案例细节以 research 为准 |
| **版本注** | **定义/价值拆读册**：[`fde-definition/`](fde-definition/)（建议先读 [`04-one-pager.md`](fde-definition/04-one-pager.md) → 01–03 → 再回本节 §2/§3）；**部门内部业务 / 四类产品能力见 [`fde-definition/06-internal-business-four-capabilities.md`](fde-definition/06-internal-business-four-capabilities.md)**；本文件仍为总册，拆读版不另立政策 |

---

## 0. 与 CodeG Option A 的关系（文首声明）

> **CodeG Option A（调度底座 × 数字员工 × 项目编队）不变，不改五条产品赌注。**  
> 本方案是 **FDE 运营层 + Ontology Lite + Demo 框架**，可映射为 CodeG 的项目/HITL/证据扩展与模板市场，**不要求换栈**。

| 对照项 | CodeG Option A（[`../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md`](../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md)） | 本方案（FDE 轨道） |
|--------|--------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------|
| 定位 | 个人 FDE 多智能体**工作台底座**（Project / Delegate / HITL / attempt 隔离 / 固定壳） | 同一底座之上的 **B 端落地作业手册 + 语义层 + Demo 脚手架** |
| 五条产品赌注 | **全部保持**（项目一等公民、默认 Delegate、HITL 三分绑 attempt、attempt 隔离、固定壳先于开放 UI） | **不改写、不否决** |
| 架构 | Option A：协作层叠云端+本机双运行时；Option B 仅对照 | **不换 Cordis/DSH**；增量能力叠在 Option A 对象上 |
| 增量映射 | — | Ontology Lite → 新模块；Demo Kit / 行业模板 → 模板市场/数字员工包；回灌队列 → 产品化候选；多 runner → 执行器适配器 |
| 明确禁止 | — | 推倒 Option A；把本方案写成「换栈理由」 |

**一句话：** CodeG 管「如何调度与验收」；本方案管「FDE 在客户现场如何把模型变成可治理的业务动作，并用本体与 Demo 框架规模化复制」。

> **工具栈修订不改变五条产品赌注。** Ontology / FDE 角色 / Agent 工具的加厚均为 Option A 之上的运营层与执行器矩阵，**不换 Cordis/DSH**。

---

## 0.1 六条综合论点（全文锚点）

| # | 论点 | 后文落点 |
|---|------|----------|
| 1 | **FDE = 生产结果所有权 + 产品回灌 + 动作闭环/write-back**，不是售前外包；企业买的是「替人把活干完」 | §2、§3 |
| 2 | 中国用驻场/交付/方案架构完成同类工作；**对内统一 FDE 模型** | §2 |
| 3 | **无本体/语义层则 Agent 在企业数据上不可治理** | §4 |
| 4 | **工作台 = 控制面；coding agents = 执行器；OpenClaw = 可选通道** | §5、§6 |
| 5 | **Ontology Lite 双用途**：客户域 + FDE 工作知识（≈ CodeG 对象） | §4、§5 |
| 6 | **Demo 框架必须模块化 + 金标 + 门禁** | §7 |

---

## 1. 背景与问题

**结论：** B 端 AI 落地的瓶颈已从「有没有模型」转为「**能否把模型接入业务决策与系统动作**」。模型 alone 不等于业务动作；缺语义层与交付所有权时，Agent 在企业数据上不可治理、不可验收、不可复制。FDE 作业模型 + Ontology Lite + 可复制 Demo 框架，是当前必要配套——不是「再写更长 Prompt」的替代品。

### 1.1 B 端鸿沟：模型 ≠ 业务动作

| 表象 | 根因 | 后果 |
|------|------|------|
| Demo 效果惊艳，试点半年不动 | 无生产所有权、无 HITL/写回边界、无 eval | 预算消耗在「看起来智能」 |
| 同一指标人机各说各话 | 无权威口径（语义层/本体） | Agent 幻觉与仪表盘冲突叠加 |
| 现场定制堆人天 | 无回灌、无模板、知识死在工单 | 规模不经济；PMF 被掩盖 |
| 任意 SQL / 任意发邮件 | 无动作白名单与权限策略 | 无法过安全与合规门禁 |
| 岗位名碎片化（驻场/交付/方案…） | 缺少统一作业模型与度量 | 招聘、培训、复盘口径混乱 |

### 1.2 为何现在必要（FDE + 本体）

1. **头部厂商已把 FDE 产品化进招聘与交付叙事**：Palantir 开创用语；OpenAI / Anthropic / Scale / Databricks 公开同名或近名角色（详见 §2 与 research matrix）。  
2. **语义/本体层成为 Agent 接地共识**：Palantir Ontology（语义+动能+安全）；Salesforce / Snowflake / Fabric 均在推「Agent 可消费的语义层」——明确建议禁止生数据直接喂模型（详 §4）。  
3. **中国市场职责同构、名称异构**：商汤等已用 FDE 品牌；多数用驻场/交付/方案架构完成同类三角（场景共建、系统集成、能力沉淀）。对内统一 FDE 模型，对外可双语话术。  
4. **与 CodeG 时间线对齐**：Option A 已定调度/HITL/证据底座；缺的是客户域建模、行业 Demo 模板与多 coding-agent 执行器矩阵——本方案补齐运营层，不换底座。

### 1.3 本方案回答的问题

> 作为 FDE，如何在 B 端做 AI 应用落地；日常如何工作；工作台应具备哪些能力；如何结合 Claude Code / Codex / Grok Build / OpenClaw / 对话 Grok；如何用本体论梳理自身工作与客户域；快速 Demo 与基线 AI 能力框架应具备哪些模块。

---

## 2. FDE 定义与美中对照

> **拆读版**（领导可读短文，可单独转发）：[`fde-definition/`](fde-definition/) — 建议 [`04-one-pager.md`](fde-definition/04-one-pager.md) → [`01-what-is-fde.md`](fde-definition/01-what-is-fde.md) / [`02-capability-four-pillars.md`](fde-definition/02-capability-four-pillars.md)；**部门内线四类产品能力 → [`06`](fde-definition/06-internal-business-four-capabilities.md)**；本节为总册完整表述，**勿删**。

**结论：** FDE = **嵌入客户、从 discovery 到 production rollout / adoption·handoff 全程负责，写生产代码，并把现场模式沉淀成产品能力（回灌）**。能力公式 = **懂技术 + 懂业务 + 能开发 + 能交付**（四象限缺一不可）。与 Solutions/Sales Engineer（售前）和纯咨询（卖建议与人天）的分水岭是 **生产结果所有权 + 产品回灌**——不是「会答题」，是「**替人把活干完**」（生产采用 + 可度量工作流影响 + 可验收动作闭环/write-back）。中国多数不用「FDE」头衔，但职责三角同构——方案对内统一称 FDE，对外可用「前线部署 / 驻场交付」。

> 定义/价值/度量三角加厚依据：[`../../research/fde-definition-value-deep-2026-10-15/`](../../research/fde-definition-value-deep-2026-10-15/)（`01-job-definition` / `02-capability-profile` / `03-value-proposition`）。日课与工具矩阵仍引用角色+工具加深包，本节不重复。

### 2.1 定义与角色光谱

| 概念 | 含义 | 典型出处 |
|------|------|----------|
| **Forward Deployed Engineer (FDE / FDSE)** | 嵌入客户现场（或深度远程），**own** discovery → technical scoping → system design → build → **production rollout** → adoption/handoff；写生产代码；回灌 playbooks/patterns | Palantir 开创（内部常称 **Delta**）；OpenAI / Anthropic / Scale / Databricks 采用 |
| **Forward Deployed AI Engineer / Frontier Agents Engineer** | FDE 的 GenAI 特化：LLM 工作流、Agent、连接器、eval harness、安全部署 | Palantir；Scale FDAIE / Frontier Agents Eng |
| **Solutions / Sales Engineer** | 偏售前：演示、PoC、方案设计；**不以**生产运维所有权为主 | 多数 SaaS；**≠ FDE** |
| **Technical Deployment Lead / Deployment Strategist** | 与 FDE 配对：SOW、干系人、价值度量/ROI 前后测 / 交付扩张；**Anthropic TDL JD 明文：won't write production code** | Anthropic TDL；Scale AI Deployment Strategist |
| **中国近义名** | 驻场/交付/方案架构/实施顾问；**商汤猎聘官方职衔「FDE 工程师」**（A 级锚点）；第四范式「AI 建设服务」；阿里云「MaaS 解决方案架构师（政企）」且 JD 写**协同 FDE 团队** | 同构职责、异构名称；**SA 默认 ≠ FDE** |

| 维度 | FDE | Solutions / SE | 传统实施顾问 / 纯咨询 |
|------|-----|----------------|----------------------|
| 生命周期重心 | 售后→**生产**（discovery→rollout→adoption） | 售前→早期售后 | 按项目人天交付 |
| 写生产代码 | **高**（多数时间；含 MCP·Agent·连接器） | 低–中（PoC） | 可变；常一次定制 |
| 结果所有权 | **生产采用 + 工作流影响 + eval 反馈** | 赢单与方案认可 | 验收与账单小时 |
| 产品回灌 | **JD 明文义务**（Codify / patterns / product improvements） | 弱 | 弱或无（不进原厂飞轮） |

**分水岭一句话：** 售前卖「可能」；咨询卖「建议与人天」；FDE 卖「**生产里真的在干活**」，并有义务把现场模式**产品化回灌**。

### 2.1.1 四象限能力画像（可观察行为）

| 象限 | 现场能看到的行为 | 缺一则症状 |
|------|------------------|------------|
| **懂技术** | 客户边界内架构取舍：SoR、鉴权、egress、数据驻留；选 RAG vs Agent vs 规则 | 方案无法在客户网络落地；安全/IT 卡死 |
| **懂业务** | 模糊痛点钉成 **1 条可测工作流** + 成功数字 + 非目标 | 漂亮但无人用的 Demo；KPI 对不上赞助人 |
| **能开发** | **亲手**写/审生产代码；交付连接器、Agent、MCP/tools、**写回路径** | 变成售前/顾问；所有权断裂 |
| **能交付** | eval 门 + launch criteria；影子→正式；培训/handoff；**回灌条目**写出 | 笔记本 PoC 永不 Harden；无采用 |

### 2.2 美国：具名公司与角色（压缩）

| 公司 | 公开职衔 | 核心做法 | 成功度量（公开表述） |
|------|----------|----------|----------------------|
| **Palantir** | FDSE / Forward Deployed AI Engineer；内部 **Delta**（相对 Product Dev） | 嵌入客户；定制应用 + LLM 工作流；Ontology 上动作与治理；单客户多能力 | 任务级 outcome；「changes how they operate」；平台采用 |
| **OpenAI** | Forward Deployed Engineer（含 Healthcare） | **own** discovery→scoping→design→build→**production rollout**；Healthcare 另含 evaluation→productionization→**adoption & handoff**；差旅可至约 **50%**；Codify playbooks | **production adoption + measurable workflow impact + eval-driven feedback**（度量三角） |
| **Anthropic** | FDE（Applied AI）+ **Technical Deployment Lead** | FDE **写生产码**（含 **MCP servers / sub-agents / skills**）并 codify patterns；**TDL 明文不写生产码**，管 SOW/干系人/ROI 前后测 | 生产安全可靠；可复用交付资产；平台反馈 |
| **Scale AI** | Forward Deployed AI Engineer / **Frontier Agents Engineer**；**AI Deployment Strategist** | 客户边界内集成管道/Agent；eval harness + HITL；Strategist 管多工作流 SLA/扩张 | 业务影响；golden eval；可复用模式回平台 |
| **Databricks** | Forward Deployed Engineer（Professional Services） | 数据+AI 生产化；参考架构；赋能客户自运营 | 客户采用与架构对齐（**岗位页 2026-10-15 已 404**，职责以曾公开招聘快照为准，见附录） |
| **Salesforce / Snowflake / Microsoft** | 偏 SA + **语义层产品化**（非强 FDE 品牌） | Data 360 DMO/Data Graphs；Snowflake Semantic Views；Fabric Ontology Preview | 一致指标、Agent 准确率、治理 |

> 详表与 URL：[`../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md`](../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md)；定义加深：[`../../research/fde-definition-value-deep-2026-10-15/02-capability-profile.md`](../../research/fde-definition-value-deep-2026-10-15/02-capability-profile.md)、[`sources.md`](../../research/fde-definition-value-deep-2026-10-15/sources.md)。

### 2.3 中国：具名公司与角色（压缩）

| 公司/类型 | 角色/服务名 | 核心做法 | 公开案例线索（置信度） |
|-----------|-------------|----------|------------------------|
| **商汤科技** | **FDE 工程师**（猎聘商汤官方 HR，**A 级职衔锚点**）；大装置亦称前线部署/驻场 | 部署联调上线、根因修复、沉淀手册/规范、培训；闭环推研发；强调 ≠ 售前外包 | 猎聘 JD（A）；铁一院/规资/南网等（品玩 2026-06；**媒体源 C，可引场景，案例 ROI% 勿当合同 KPI**）— 职衔 **高** / 案例数字 **软** |
| **第四范式** | **AI 建设服务**（安装/升级/数据融合/系统融合/建模/定制/**投产**） | 座谈摸底→方案→开发测试→**生产上线**；上下游对接 | 来伊份等（官网服务菜单，A）— **高** |
| **阿里云** | **MaaS 解决方案架构师（政企）**；JD 文案写**协同 FDE 团队**推动落地 | 场景路径、模型选型、POC/效果评估、沉淀最佳实践、反馈产品；**显式与 FDE 分工，SA ≠ FDE** | careers 搜索高亮（A−；SPA 正文未完整拉取）— **中–高** |
| **华为云 iDME** | 工业软件底座 + 实施/伙伴 | 工业本体/数字主线：对象模型 + 图谱联接 + 模板库 | 产品文档场景— **高（产品）/中（具名客户细节）** |
| **火山引擎** | 解决方案 / Agent 平台（HiAgent、AgentKit、Agent Mesh） | 私有化 Agent 全生命周期、MCP/RAG/观测 | 公开偏能力白皮书；行业案例需按项目核实— **中** |
| **百度智能云** | 千帆 + 方案交付 | Multi-Agent、文档解析、工具调用 | 平台页行业方案— **中** |
| **腾讯等** | 解决方案架构师 / 交付 | 行业方案 + 集成商生态 | 本 research 包**缺少**可独立深核标杆长文— **低（缺口，勿编造）** |

### 2.4 中国命名对照（写进对外话术）

| 对外话术（CN） | 对内统一 | 备注 |
|----------------|----------|------|
| 前线部署工程师 / FDE | **FDE** | 利于对齐国际人才与作业模型；商汤已用职衔 |
| 驻场工程师 | FDE（驻场履约） | 强调物理嵌入 |
| 交付工程师 | FDE（履约阶段） | 易弱化产品回灌——**方案显式要求回灌队列** |
| 解决方案架构师 | SA；可与 FDE **配对**（阿里已写协同 FDE） | 偏设计与售前时**不要等同** FDE |
| 建设服务（第四范式） | FDE 能力外包/原厂服务包 | 生产上线链完整；服务菜单 ≠ 个人职衔 |
| 实施顾问 | Implementation | 常缺生产代码深度 |

### 2.5 美中对比（对 Mack 的含义）

| 维度 | 美国具名 | 中国具名 | 含义 |
|------|----------|----------|------|
| 品牌头衔 | OpenAI/Anthropic/Scale/Palantir **直接叫 FDE**；Palantir 内部 **Delta** | 商汤 **叫 FDE**（猎聘 A）；多数叫交付/驻场/SA | **对内统一 FDE，对外可双语** |
| 写生产码 | JD 明文高占比；**FDE 写码 / TDL 不写码**（Anthropic） | 商汤 FDE、范式建设开发上线；阿里 SA 偏方案+**协同 FDE** | 中国「SA」≠「FDE」除非 JD 含生产码与上线所有权 |
| 配对 | TDL / Strategist | 售前 SA + 交付/FDE；阿里明示协同 FDE | 组织设计可抄 Anthropic 拆分 |
| 产品回灌 | Codify / patterns / product feedback（JD 义务） | 商汤闭环改进；范式偏项目交付；阿里反馈产品迭代 | **回灌强度是分水岭**；工作台要有回灌队列 |
| 度量话术 | **adoption / workflow impact / eval**（OpenAI 三角） | 验收、见效、ROI 叙事、续费 | 方案用 OpenAI 三角作主度量；ROI% 媒体数字软写 |
| 语义/本体 | Ontology / Semantic Layer 产品化强 | 工业本体/图谱/主数据强；Palantir 式「动作本体」公开少 | **Ontology Lite + 动作白名单**是差异化（详 §4） |
| 合规 | GRC/安全嵌入 FDE | 私有化、国产化、数据不出域更硬 | Demo 框架要有**私有化/本地运行剖面** |
| 生态 | 原厂 FDE 稀缺高价 | 伙伴/集成商放大 | **模板化 Demo** 比堆人更杠杆 |

### 2.6 招聘信号与面试环（软补充）

| 阶段 | 测什么 | 常见失败 |
|------|--------|----------|
| Recruiter / HM screen | 「为何 FDE 而非纯 SWE」动机；是否接受全程负责+回灌 | 只谈旅行/高薪 |
| Practical coding | API 集成、backoff、脏数据、小 RAG、写回路径 | 只会 LeetCode 抽象题 |
| System / architecture | VPC RAG、合规、agent eval 框架 | 忽略客户约束 |
| Decomposition / scoping | 模糊业务题 → MVP + 度量 | **未澄清就架构** |
| Customer simulation | 角色扮演干系人压力 | 技术甩锅、不懂取舍 |

**时长（从业者报道近似，非雇主官方，B 级）：** Palantir 约 4–5 周；OpenAI 约 3–4；Anthropic / Scale 约 2–3。公开差旅：OpenAI 最高约 50%；Anthropic 约 25–50%。**Staffing 比率未公开，勿写成确定 KPI。** 详 [`../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-role-deep.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-role-deep.md) §2。


## 3. FDE 作业模型

> **拆读版**（度量三角 / write-back / G1–G7 / 红线）：[`fde-definition/`](fde-definition/) — [`03-value-get-work-done.md`](fde-definition/03-value-get-work-done.md) · [`04-one-pager.md`](fde-definition/04-one-pager.md) · 可选 [`05-faq.md`](fde-definition/05-faq.md)；**内线作业对象 / 四类产品能力 → [`06`](fde-definition/06-internal-business-four-capabilities.md)**；本节为总册完整表述（含日课/WIP），**勿删**。

**结论：** FDE **own** 全弧：discovery → technical scoping → system design → build → **production rollout** → **adoption / handoff**（OpenAI careers 原文结构）；并把模式 **Codify** 为 playbooks / building blocks 回灌产品。单位产出不是账单人天或「会答题」的 Demo 惊艳度，而是买方付费对象——**生产里被采用的、可度量的工作流改变 + 可验收动作闭环（write-back）**。成功度量三角 = **production adoption + measurable workflow impact + eval-driven feedback**（OpenAI）。用统一生命周期、角色三角、日/周节奏、必交付工件与 WIP 限额运行每一次 engagement；红线写死——无回灌不做长期驻场、Demo≠Prod、无金标 eval 不上生产、禁止影子 IT 上客户 PII。

> 价值主张与验收门加厚：[`../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md`](../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md)。日课节奏见 §3.3（角色+工具加深包）；本节不改日课表。

### 3.1 角色三角与配对

| 角色 | 职责（Own） | 硬边界 | 对标 |
|------|-------------|--------|------|
| **FDE（主交付）** | discovery、架构、**写生产代码**、集成（含 MCP·Agent·写回）、eval、上线、证据包、回灌候选 | **写码方**；不单独拍板商业合同/定价 | OpenAI/Palantir/Anthropic FDE；Scale FDAIE；商汤 FDE 工程师 |
| **交付/部署负责人（可兼任）** | SOW→production 范围、干系人、**value measurement / ROI 前后测**、变更管理 | Anthropic **TDL JD 明文：won't write production code**；alongside FDEs who build | Anthropic **Technical Deployment Lead**；Scale **AI Deployment Strategist** |
| **客户侧 Sponsor + 业务 Owner + IT 对口** | 数据权限、验收签字、采用推动 | 缺一则 engagement 不可闭环 | — |

```
售前 SE / SA ──赢单/PoC──► │ 签约后
                           ▼
              ┌── TDL / Strategist ── SOW、干系人、ROI、范围变更（不写生产码）
              │
              └── FDE / FDAIE / FDSE ── 生产代码、集成、eval、上线、回灌候选
                           │
                           ▼
                    CS / 客户成功 ── 上线后健康度与扩张（可交接）
```

**组织设计：** 一人公司可「兼 FDE + 轻量 TDL」；**并行客户 ≥2** 时应显式拆「写码」与「范围/ROI」看板，否则范围腐蚀。高杠杆 = **TDL 配对 + 行业模板 + junior↔senior twinning + 回灌队列**，不是堆更多驻场。**≠ 售前 SE，≠ 纯咨询人天**（无产品飞轮则改牌成外包）。

### 3.2 生命周期与必交付工件

| 阶段 | 活动 | 必交付工件 |
|------|------|------------|
| **1. Discovery** | 访谈业务与 IT；找「真正值钱且可测」的问题；系统/数据地图 | 问题陈述+约束一页纸、成功指标草案、系统/数据地图、风险清单 |
| **2. Scope / SOW** | MVP 边界、权限与数据约束、退出条件 | SOW、成功数字、非目标、禁区、退出条件 |
| **3. Model** | 客户轻量本体工作坊（CQ 驱动） | **Ontology Lite v0**、CQ 列表（5–15）、权限矩阵（详 §4） |
| **4. Demo / PoV** | 真实或脱敏样本验证价值 | 可点击 Demo + 金标集 + **「非生产」声明** + 环境剖面标签 |
| **5. Integrate** | 连接器、身份、写回、Agent 工具边界 | 连接器契约、工具白名单（⊆ ActionType）、身份方案 |
| **6. Harden** | 幂等、观测、runbook、eval、成本与延迟 | Eval 报告（绑 `ontology_version`）、runbook、成本模型、观测面板 |
| **7. Adopt** | 培训、变更管理、影子运行→正式运行 | 培训材料、影子运行结果、HITL 日志 |
| **8. Handoff + Productize** | 移交；模式抽成 playbook / 模板 / 产品特性 | 运维手册、**≥1 回灌条目**/里程碑（进 CodeG 模板库） |

### 3.2.1 作业对象双轨（指针）

| 轨 | 说明 |
|----|------|
| **外线** | B 端客户 engagement（本文 §3 主体） |
| **内线** | 部门内部业务 × **四类产品能力**（智能客服 / 知识库 / 语音 / AI Coding 工具）；同一所有权弧与 G1–G7；赞助人 = 部门产品 Owner / 业务方；回灌进**对应品类/产品** backlog |

部门示例产品名（如智能客服→阡陌、AI Coding→飞码）仅示例，**不以品牌为主轴**。切面表、红线补充与软写纪律见拆读 [`fde-definition/06-internal-business-four-capabilities.md`](fde-definition/06-internal-business-four-capabilities.md) —— **不推倒** CodeG Option A；不在此重复长表。

### 3.3 日课与周节奏（默认）

**日课硬结构：** Standup（书面阻塞）→ Scope（可度量切片）→ Build → Eval（数字）→ Stakeholder（取舍记录）。

| 时段 | 活动 | 输出工件 |
|------|------|----------|
| 早 | 客户 standup（含对方） | 阻塞清单：事项 / Owner / 日期（**1h 内发出**） |
| 午前 | 模糊需求 → 可度量目标 | 范围切片：1 工作流 + 成功数字 + 非目标 |
| 中 | 对着真实系统编码/集成 | PR / 配置 / 连接器；attempt 日志 |
| 午后 | Eval 金标跑分 | 分数 + 仍失败案例（非技术可读摘要） |
| 傍晚 | 干系人取舍对话 | 决策记录：日期/选项/风险/谁拍板 |
| 不定 | 差旅 / 现场 | 现场笔记进项目资料区（非只在脑中） |

**编码 vs 对话：** 正常周约半半；Discovery/动荡上线 → 对话主导；中期建造 → 2–3 块连续编码。写作量常被低估。

| 日（履约中段） | 焦点 |
|----------------|------|
| 一 | 周计划对齐 SOW 里程碑；刷新风险 |
| 二–三 | 深建造 + 集成；并行 eval 扩样 |
| 四 | 安全/IT 对接；权限与写回演练 |
| 五 | Demo/验收预演；**回灌条目草稿**；下周阻塞预报 |

### 3.4 技能矩阵（Great vs Average）

| 维 | Average | Great（现场与面试都筛这个） |
|----|---------|------------------------------|
| **编码** | 能写服务 | 在**客户网络**里写：幂等、鉴权、半文档 API、可回滚 |
| **系统** | 知道云产品名 | 画清 SoR、数据驻留、egress、SSO（SAML/OIDC） |
| **LLM/Agent** | 会调 API | 工具边界、RAG 权限、**golden eval**、成本/延迟权衡 |
| **领域** | 听得懂行话 | 钉成对象/动作/指标（Ontology Lite，§4） |
| **变更管理** | 做培训 PPT | 影子运行→正式、绕过检测、赞助人管理 |
| **沟通** | 会开会 | **1 小时内书面阻塞纪要**；技术真相进非技术房间 |
| **产品感** | 完成工单 | 每次交付抽出 playbook / 模板候选 |

**培训启示：** 内部 FDE 训练练「澄清脚本 + eval 门 + 干系人取舍」三件套，而非只练 prompt。

### 3.5 成功度量（度量三角 + 上线门 + 飞轮）

**买方价值句（可硬写）：** 企业买的不是「AI 会答题」，而是「**AI 替人把活干完**」——生产采用 + 可度量工作流影响 + 可验收动作闭环（write-back）与 eval 门。

#### 3.5.1 OpenAI 成功度量三角（主度量）

| 角 | 定义 | 说明 |
|----|------|------|
| **Production adoption** | 目标用户群在生产环境按设计频率使用 | ≠ 试点账号点开 Demo；≠ 账单人天 |
| **Measurable workflow impact** | 事先约定工作流 KPI：基线 → 后测（时延、吞吐、人工小时、差错率、完成率等） | 对齐 Anthropic TDL pre/post ROI 报表格式 |
| **Eval-driven feedback** | 金标/客户特定 benchmark 失败案例进产品与模型路线图 | OpenAI：changes product and model roadmaps |

#### 3.5.2 上线门（对齐 OpenAI Healthcare）

| 门 | 定义 | 对齐 |
|----|------|------|
| Customer-specific **benchmarks** | 客户域金标集与阈值阈值 | Healthcare FDE JD |
| **Acceptance criteria** | 书面可签字的通过/失败阈值 | 同上 |
| **Launch readiness** | 观测、升级/回滚、HITL、escalation、runbook 齐备 | 同上 |
| **Handoff** | 客户（或支持方）能在无原厂 FDE 盯屏下运行；培训完成 | Healthcare：adoption & handoff 是端到端一段 |

#### 3.5.3 双轨 + write-back 条件 + 飞轮

| 轨 | 度量 | 说明 |
|----|------|------|
| **业务** | 采用 + 工作流 KPI + **动作闭环/write-back**（经治理写入 SoR/工单/审批流；幂等·权限·审计） | **write-back 闭环是业务轨必要条件**；缺写回则仍是「会答题的 Demo」 |
| **工程/AI** | 金标 eval、回归门、成本/延迟、HITL SLA、launch readiness | 无金标则不得宣称「已生产落地」 |
| **产品飞轮** | 回灌条目（playbook / 模板 / MCP·skill / 平台缺口）≥1/里程碑；模板复用次数 | 区分「人天项目」与「FDE」；OpenAI Codify / Anthropic patterns / Scale reusable |

**「替人把活干完」操作化：** 采用 ∧ 工作流指标 ∧ 写回/动作闭环 —— 缺一则未交货。

#### 3.5.4 验收门 G1–G7（压成 DoD 表）

| 门 | 定义 | 对齐雇主 |
|----|------|----------|
| **G1 Production adopted** | 生产按设计频率使用 | OpenAI adoption |
| **G2 Workflow metric** | 基线+后测的工作流 KPI | OpenAI impact；TDL pre/post |
| **G3 Eval gate** | 金标达 acceptance threshold；失败可追溯 | Healthcare；Scale golden |
| **G4 Action / write-back** | 经治理写入 SoR/工单；幂等·权限·审计 | Healthcare API/EHR；Palantir operate |
| **G5 Launch readiness** | 观测/回滚/HITL/runbook | Healthcare launch criteria |
| **G6 Handoff** | 无原厂盯屏可运行 | Healthcare handoff |
| **G7 Productize** | ≥1 回灌候选进产品队列 | OpenAI Codify；Anthropic patterns |

**健康飞轮：** 现场模式 → playbook → 产品特性 → 下一客户更少定制。  
**不健康飞轮：** 现场定制 → 工单 → 人月膨胀 → 产品无学习。

差旅公开区间：OpenAI JD「最高约 50%」；Anthropic「25–50%」；从业者均值叙事约 30–50%（**方差大，勿写成公司精确 KPI**）。Staffing 比率雇主极少公开 → **低置信，勿写成确定公式**。Scale JD「节省员工/护士数百小时/周」等为**雇主样例叙事（A 级来源、非独立审计）**，可引作「替人干完活」话术，**勿当合同 KPI**。

### 3.6 WIP 与杠杆默认

| 规则 | 默认 |
|------|------|
| **WIP** | 一人同时深潜客户 **≤2**；第三人只维护不新开 |
| **每周五** | 回灌草稿 + eval 回归 + 阻塞预报 |
| **声称可用的工作流** | 金标集 ≥ 业务同意的 N 条（先约 30–50，再扩） |
| **工具** | 只允许控制面批准的 runner；客户数据默认不出域 |
| **本体** | 先 5–15 对象 + 3–5 动作（详 §4），再谈平台选型 |
| **杠杆** | TDL 配对、模板、twinning、伙伴交付（CN）；反杠杆 = 无回灌定制、FDE 挂销售线 |

### 3.7 失败模式红线（写入方案）

**价值主张反面（可硬写）：** Chatbot PoC 失败 = 缺 **采用 ∧ 工作流指标 ∧ 写回/动作**；看起来像「会答题」，实际未「替人把活干完」。

| # | 红线 | 为何致命 | 来源级 |
|---|------|----------|--------|
| R0 | **Chatbot PoC 当交付**：准确率演示好看，无工作流嵌入/写回/采用 | 未过 G1∧G2∧G4；买方未得到可验收改变 | A：OpenAI 度量三角 |
| R1 | FDE 变成**无回灌**的定制咨询 | 无产品飞轮；规模不经济；改牌外包 | A：OpenAI/Anthropic/Scale Codify 义务 |
| R2 | **Demo = Prod**（Demo 直接当生产） | 无 harden / eval 门 / launch criteria → 事故与失信 | A：Healthcare evaluation→productionization；B：fdeinterviews |
| R3 | **跳过 Discovery** | 做漂亮但无价值的问题；CQ+成功数字签名前不开码 | A：OpenAI/Palantir/TDL |
| R4 | **单人知识孤岛** | 人走灯灭；强制 twinning + 书面轨迹 | 运营纪律 |
| R5 | FDE 汇报线挂销售、用 FDE **掩盖弱 PMF** | 「FDE trap」；成功度量 ≠ 赢单 | B/C：从业者讨论，**软写** |
| R6 | 现场学习死在工单，**进不了产品** | 与 R1 同构；每里程碑 ≥1 回灌条目 | A：回灌义务 |
| R7 | **无金标 eval 上生产** | 不可回归、不可治理 | A：OpenAI/Scale |
| R8 | Agent **任意 SQL / 任意外发 / 无界写回** | 合规与安全不可过；须 Action 白名单 + HITL | 与 §4/§6 交叉 |
| R9 | **影子 IT**：个人 ChatGPT/Grok 粘贴客户 PII | 批准工具清单 + 数据分级 | 运营纪律 |
| R10 | **无本体绑定**的工具面 | 裸 SQL + 裸 HTTP = 不可治理（详 §4.8 / §6） | 与 §4 交叉 |
| R11 | **沙箱当验收** | 厂商沙箱 ≠ 业务 HITL | 运营纪律 |
| R12 | **无限账户 / 无 WIP**；成功变成 **utilization 人天** | 倦怠、质量塌；咨询指标冒充产品 FDE | B：fdeinterviews |

详 [`../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-role-deep.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-role-deep.md)；价值反模式加深 [`../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md`](../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md)。

---

## 4. 本体论

**结论：** 企业本体不是「知识图谱可视化」或「更长 Prompt」，而是**可被 Agent 消费的运营世界模型**：名词（对象/关系/指标）+ 动词（受治理动作）+ 绑定（真实系统）+ 安全（对象/动作级）+ 版本（变更与 eval）。FDE 日常 = **CQ 驱动抽知识 → 压成 Ontology Lite → 只经本体暴露工具给 Agent → 金标/轨迹门禁上线**。首期只承诺 Ontology Lite；**无本体/语义层，Agent 在企业数据上不可治理。**

> CodeG 继续作为调度/HITL/attempt/证据底座（Option A 五条赌注不变）。  
> Ontology Lite 是 FDE 轨道上的**客户域世界模型 + 动作契约**；以项目内域包形式挂载，经 snapshot 注入 runner。  
> 数字员工编队共享同一客户域投影，禁止每员工私藏第二套指标/动作定义。

### 4.0 加深声明

| 项 | 说明 |
|----|------|
| **本轮性质** | Ontology 章**加深**（建设方法、抽取、Agent 机制、日常控制点）；**不是**新开换栈方案 |
| **深潜调研包** | [`../../research/ontology-enterprise-agent-deep-2026-10-15/`](../../research/ontology-enterprise-agent-deep-2026-10-15/)（`handoff-to-方案撰写.md`、`build-and-extract.md`、`ontology-for-agents.md`、`daily-ops-and-control-points.md`、`sources.md`） |
| **并列旧包** | [`../../research/fde-ontology-b2b-ai-2026-10-15/`](../../research/fde-ontology-b2b-ai-2026-10-15/) 仍管 FDE 美中矩阵与浅层对照；本包专管 Ontology 深潜 |
| **硬约束** | **不推翻 CodeG Option A**；不换 Cordis/DSH；不与 Foundry 功能一对一承诺 |

### 4.1 四层辨析（禁止混为一谈）

| 层 | 是什么 | 不是什么 | 代表 |
|----|--------|----------|------|
| **A. 运营本体（Operational Ontology）** | 对象+属性+链接（**语义**）**与** 动作+函数+动态安全（**动能**）；人与 Agent 共用决策世界 | 不是纯知识图谱展示层，也不是薄 BI 语义层 | Palantir Foundry Ontology / AIP |
| **B. Semantic Layer（指标/实体语义层）** | 物理表之上的实体、维度、指标、同义词、已验证查询；服务 BI + Text-to-SQL + Agent | 通常**弱动能**（少「写回动作」一等公民） | Snowflake Semantic Views；Salesforce Data 360 DMO/Data Graphs；dbt/OSI |
| **C. Knowledge Graph** | 实体关系图 + 推理/检索；偏事实与关联 | 不一定含动作治理与写回 | 阿里商品图谱、蚂蚁风控图谱、华为 LinkX 主题图谱（**公开材料多为二手/产品叙事，中等置信**） |
| **D. MDM / 业务对象模型** | 核心业务对象统一身份与属性 | alone 不够支撑 Agent 决策闭环 | 传统 MDM；华为 iDME 数据建模引擎接近「工业对象模型」 |

**与 Semantic Layer 的关系（硬纪律）：** Ontology ≠ 薄 Semantic Layer。Palantir Architecture Center 明文：Ontology **不是**「薄语义层」；是 **data + logic + action + security** 四元一体。可说「谱系相邻、人机共用业务定义」；**不可**说 Semantic Layer ≡ Palantir Ontology。

**硬句（给方案与销售）：**

> Semantic Layer 解决「同一个数」；运营本体还解决「同一套可允许的做事方式」。  
> B 端 Agent 生产落地需要后者；前者是必要但不充分条件。  
> 若客户只有 Snowflake/dbt 语义层，FDE 的增量交付是：**在其上补 ActionType + HITL + 审计 + eval 轨迹**，即 Ontology Lite L2，而不是宣称已有 Foundry。

**工作定义（本方案采用）：**

> **业务运营本体** = 业务对象（名词）+ 关系 + 权威指标/规则 + **可治理动作（动词）** + 权限策略；绑定真实系统数据；供人与 Agent 共用。  
> 实现上从 Ontology Lite 起步，**不必**首日复刻 Foundry 全量。

### 4.2 美中落地要点（可硬写摘要）

| 来源 | 要点 | 写法纪律 |
|------|------|----------|
| **Palantir** | Ontology = 语义（object/link）+ 动能（action/function/security）；AIP：LLM **申请**工具、运行时按调用者权限执行；Ontology MCP 对外消费 / Palantir MCP 改结构 | 可硬写机制；**勿**承诺功能一对一 |
| **Salesforce Data 360** | DLO→DMO→Data Graphs；**禁止生 DLO 喂 LLM**；按 access pattern 拆图；OSI 语义互通叙事 | P50&lt;200ms 等为**厂商工程叙述**，软写 |
| **Snowflake** | Semantic View + descriptions + verified queries；小域起步（约 5–10 表）；VQ **勿**补烂模型 | AtScale 准确率跃迁等为**第三方/厂商引用**，非本包复现 |
| **Microsoft Fabric Ontology** | 实体/关系/指标/规则/绑定；Ontology-as-MCP；内置 Agent proposal-first | **Preview**；旧体验退役标注 **2027-01-31**；写「可参考模式」非「已全面 GA」 |
| **华为 iDME** | xDM-F / LinkX-F / BoX；逆向建模、入图、工业模板 | 强对象与追溯；**Agent 动作治理公开少** → 中国路径常需**自补动作层** |
| **蚂蚁/阿里等图谱** | 金融风控/商品语义等公开分享与转述 | **中等置信**；勿当作我方审计过的一手流程；「买了图谱 ≠ 有运营本体」 |
| **第四范式等** | 「数据融合」工程路径 = 本体前置的实务形态 | 即使不叫 Ontology |

### 4.3 Ontology Lite 范围（首期可交付）

```
EntityType { name, props[], keys[], description, synonyms[] }
LinkType   { from, to, name, cardinality }
Metric     { name, definition, owner, synonyms[], verified_queries[] }
ActionType { name, params[], preconditions[], side_effects[], permission_policy, hitl, audit, binding }
Binding    { entity|metric|action → connector.field | API | MCP tool }
Version    { semver, changelog, approved_by, demo_vs_prod }
```

| 首期做 | 首期**刻意不做** |
|--------|------------------|
| ≤15 实体、≤10 动作的客户 v0；≥5 CQ；≥1 Owner 签字 | 全量数字孪生引擎 |
| SoR 绑定 + 版本冻结/变更评审；`demo_vs_prod` 标签 | 跨全球对象事务写回中枢 |
| 动作白名单 + HITL + 审计；eval 绑 `ontology_version` | 自研图计算平台 |
| 行业模板骨架 + 客户扩展包 | 与 Palantir 功能一对一对等承诺 |

**v0 规模闸门：** ≤15 entities、≤10 actions、≥5 CQ、≥1 Owner 签字；无 CQ 不做大模型。

### 4.4 客户域 vs 工作本体（双用途）

| 用途 | 内容 | 与 CodeG |
|------|------|----------|
| **(a) FDE 工作知识** | CustomerAccount、Engagement、Attempt、Artifact、Evidence、HITL Gate、Playbook、Risk | **≈ CodeG 核心对象**（Project / Attempt / HITL / Evidence） |
| **(b) 客户域建模** | 客户业务对象/指标/动作/SoR 绑定 | **可插拔域包**；经 snapshot 注入 runner；Demo 与 Agent 的真相源 |

| Ontology Lite | CodeG（轻映射，不换栈） |
|---------------|------------------------|
| 客户域包 | 项目内可插拔域包 / 模板市场 |
| FDE 工作本体 | Project / Attempt / HITL / Evidence（已有） |
| Action + HITL | HITL 三分轨绑 attempt |
| ontology snapshot 注入 | runner 启动上下文 |
| 版本 + eval | 证据包与回归门 |

**硬规则：** 客户本体与 FDE 工作知识**分库/分命名空间**（失败模式 F10）；数字员工编队**共享同一客户域投影**，禁止每员工私藏第二套指标/动作定义。

### 4.5 企业怎么建（方法谱系）

**结论先行：** 建本体是**混合工程**（CQ + middle-out + LLM 提议/人裁定），不是「画一张图」；默认按场景切片，**何时不做全量**必须写进 engagement。

#### 4.5.1 方法谱系

| 方法 | 做法 | 何时用 |
|------|------|--------|
| **Competency Questions（CQ）** | 列出本体必须能回答/支持的问题与任务；CQ → 验收/金标 | **任何 Lite v0 的入口**；无 CQ 不做大模型 |
| **Middle-out** | 从共识核心概念起，向上抽象、向下特化 | 企业最常用；避免纯 top-down 空转或纯 bottom-up 噪音 |
| **Top-down** | 行业标准起步（FIBO、schema.org、BoM/SysML 模板等） | 强监管/强标准行业；再本地化 |
| **Bottom-up** | 从 ER/表/日志/文档 LLM 提议候选 | **加速发现**；**人裁定权威**（LLM proposes, humans ratify） |
| **Reuse** | 导入 RDF/OWL；行业 BoX；C360 DMO；从 BI semantic model 生成草稿 | 降低冷启动；**不替代治理** |

**企业 AI 落地默认：** middle-out + LLM bottom-up 加速 + Owner top-down 权威。

#### 4.5.2 产品形态谱系（动能强度）

| 形态 | 动能（写回） | Agent 典型消费 | 代表 |
|------|--------------|----------------|------|
| **运营本体** | **强**（Action + submission criteria） | Query objects + Apply actions；Ontology MCP | Palantir Foundry + AIP |
| **Semantic Layer** | **弱**（偏读） | Cortex Analyst/Agents；MetricFlow | Snowflake；dbt |
| **Agent Context Graph** | 中–弱 | 预组装 Data Graph 上下文 | Salesforce Data 360 |
| **Fabric Ontology** | 中（Preview） | Ontology-as-MCP；data/ops agents | Microsoft Fabric IQ |
| **工业数字主线** | 偏数据服务/追溯 | 索引/追溯；动能公开少 | 华为 iDME |
| **知识层（KL）** | 策略内动作 | 共享意义 + memory 轨迹 | Neo4j Knowledge Layer 叙事 |

#### 4.5.3 何时不做全量本体（硬闸门）

1. 问题可用单一 SoR + 约 5 个固定 API 解决 → 先 Action 白名单，不建企业图谱。  
2. 无业务 Owner 愿为口径签字 → **停建**，改 Discovery。  
3. 目标是「一年建完全球图谱」→ 改 **按场景切片**（约 5–10 表 / 按 access pattern 拆图起步）。  
4. 只有文档没有动作边界 → 先做只读 Semantic Lite，写动作另立项。

### 4.6 RACI（语义/本体面）

**原则：** 工具/仓库**不是** A；**Business / Domain Owner 才是 A**。Steward 维护定义与变更包（R），**不**独自承担业务风险。Custodian 只部署，不偷 A。

| 活动 | Domain Owner | Steward | FDE | DE/Custodian | SME | Security | Agent Eng |
|------|--------------|---------|-----|--------------|-----|----------|-----------|
| 批准口径/指标发布 | **A** | R | C | I | C | C | I |
| 维护定义/映射/分诊 | C | **R** | R | C | C | I | I |
| 工作坊与 v0 建模 | A（范围） | C | **R** | C | **C** | C | C |
| 数据绑定/管道 | I | C | R | **R** | I | C | I |
| Action/HITL 设计 | A（风险） | C | **R** | C | C | **C** | C |
| 暴露给 Agent（投影/MCP） | A | C | **R** | C | I | **A/C** | R |
| Eval 门禁与回归 | C | R | **R** | C | C | I | R |
| 生产变更部署 | A | R | C | **R** | I | C | I |
| 跨工具冲突升级 | **A**（超时默认） | **R**（组包） | C | I | C | C | I |

角色名可本地化（如「业务 Owner / 语义 Steward」），**Owner=A / Steward=R** 原则不变。

### 4.7 抽取 playbook（四源 + 动作分级）

**结论先行：** 业务知识抽取是可执行流水线——系统/人/文档/用法四源并行 → SME 裁定 → 绑定 → 版本；**LLM 只产候选，不产权威**。

```
系统源 ──► ER/MDM/API/事件 schema ──► Object/Link 候选 + Binding
人员源 ──► 工作坊/决策树/SOP/例外目录 ──► Metric/Action/Rule 候选
文档源 ──► 词表/政策/合同 NLP 候选   ──► 术语 + 约束（人审）
用法源 ──► BI 查询日志/仪表盘/工单   ──► Verified queries + 同义词
                    │
                    ▼
           SME 裁定 → 版本化 Ontology Lite → 暴露给 Agent
```

#### 4.7.1 抽取纪律

1. **每个关键属性一个权威源（SoR）**；多源并存必须标 `context=` / `deprecated`。  
2. **名词与动词分轨**：名词→Entity/Link/Metric；动词→Action（参数、前置条件、副作用、权限、HITL、审计、binding）。  
3. **LLM 只产候选，不产权威**（proposal-first）。  
4. **指标先于同义词**：优先高质量 description + metrics/filters；勿自动灌同义词。  
5. **动作分级写入 ActionType**，而不是 Prompt。

#### 4.7.2 动作分级

| 级 | 例 | Agent |
|----|-----|-------|
| L0 只读查询 | GetAccount、SearchOrders | 默认开 |
| L1 低风险写 | 打标签、草稿 | 可开 + 审计 |
| L2 业务写 | 改价、改负责人 | HITL |
| L3 资金/合规 | 退款、关停 | 人-only 或双人 |
| L4 禁止 | 任意 SQL、任意外发 | **永不进工具表** |

#### 4.7.3 版本与黄金源

| 规则 | 做法 |
|------|------|
| Semver | MAJOR=破坏性口径；MINOR=新实体/动作；PATCH=描述/VQ |
| 冲突 | 并存时标 `canonical` vs `context=BU_X`；禁止静默覆盖 |
| 黄金源 | 每 property 一行 SoR；变更需 Owner |
| 变更门禁 | Steward 组包 → Owner 批 → Custodian 部署 → 通知消费者 |
| Eval 耦合 | 发布前跑金标；失败则拒发 |

细则与「下周最小包」YAML 形状见深潜包 [`build-and-extract.md`](../../research/ontology-enterprise-agent-deep-2026-10-15/build-and-extract.md)。

### 4.8 服务 Agent（运行时契约）

**结论先行：** 本体是 Agent 的**运行时契约与世界模型**，不是 Prompt 附录。默认工具面 = 绑定后的 Entity 读 + Action 写；无「万能 query」工具。

#### 4.8.1 机制地图

| 机制 | 做法 |
|------|------|
| **Grounding** | Agent 只见对象/语义视图/Data Graph，**禁止生表/DLO** |
| **Planning** | 计划在 objects+actions 上：`observe → decide → invoke(Action) → HITL? → commit`；非任意 SQL |
| **Tool mediation** | LLM **申请**工具；运行时**按调用者/项目权限**执行 |
| **Permissions** | 对象/属性级读 + Action submission criteria；**读 RLS ≠ 写/输出安全**（须配对 marking/输出过滤/写闸） |
| **投影** | 每 Agent 可见 entities/actions 子集；编队共享同一世界模型 |
| **Eval** | 金标/VQ/轨迹越权用例；报告写入 `ontology_version`；MAJOR 强制全量回归 |
| **MCP** | Ontology-as-MCP 对外；自定义 MCP **必须**从本体生成或校验，禁止手写第二套 |
| **多 runner** | Claude Code / Codex / Grok Build 只换执行器，**不换世界模型** |

**端到端调用链：**

```
用户意图
  → 控制面注入 ontology snapshot（版本、投影、CQ 摘要）
  → Planner 在 Entity/Action 空间拆步
  → Tool runtime：读=Query/SemanticView/DataGraph（RLS）；写=仅 ActionType + HITL
  → 证据：工具调用、对象前后快照、审批记录
  → Eval/审计挂同一 ontology_version
```

#### 4.8.2 失败模式红线（F1–F10）

| ID | 失败 | 后果 | 缓解 |
|----|------|------|------|
| **F1** | Agent **直接 SQL** 生表 | 口径漂移、越权、不可审计 | 默认拒；仅临时 break-glass + 双人 |
| **F2** | **无动作**只 RAG 问答 | 「能说不能办」 | L2 Action + HITL |
| **F3** | **有动作无权限/审计** | 无法过安全 | submission criteria + 审计字段 |
| **F4** | 每 Agent **私藏 MCP 定义** | 静默语义分叉 | 从本体生成工具；禁止手改权威 |
| **F5** | **读 RLS 未覆盖写/输出** | 外泄或误写 | 配对 marking；输出过滤；写单独闸 |
| **F6** | **VQ/金标补烂模型** | 表面分高、泛化差 | 先修 description/关系/指标 |
| **F7** | **全量上下文灌入** | 贵、慢、噪 | Data Graph 粒度；对象投影 |
| **F8** | **Demo 本体当 Prod** | 假绑定/假权限上生产 | `demo_vs_prod` 标签 + Promote 门禁 |
| **F9** | **无版本** | 无法回归 | semver + eval 耦合 |
| **F10** | **客户本体与 FDE 工作知识混库** | 权限与复用灾难 | 分 namespace |

### 4.9 成熟度 L0–L3

| 级别 | 内容 | Agent 能力 | 退出条件 |
|------|------|------------|----------|
| **L0 Glossary** | 术语表 + Owner | 几乎无 | 有人签字的名词表 |
| **L1 Objects + Metrics** | Entity/Link/Metric + Binding + descriptions | 只读问答 / 受约束 Text-to-SQL | 金标只读 ≥ 约定阈值 |
| **L2 Actions + Security** | ActionType + 权限 + HITL + 审计 | 可办事（受白名单） | 写动作影子运行通过 |
| **L3 Agent-native** | 投影/MCP/多 Agent 共享世界 + 轨迹 memory | 编队共用同一世界模型 | 跨 Agent 口径一致 + 回归门 |

**中国路径提示：** 工业对象/图谱/主数据强、公开「Agent 动能动作治理」少 → 差异化常在 **自补动作层（冲 L2）+ 权限 + eval**，而非再买一张图谱。

### 4.10 日常节奏与七控制点

#### 4.10.1 Engagement 双周切片（可压到约 10 个工作日）

| 日 | 焦点 | 出口 |
|----|------|------|
| D1 | Kickoff + 系统地图 | Owner/Steward 指名；禁区 |
| D2 | CQ 工作坊 | CQ≥10；名词动词墙 |
| D3 | 候选模型 | Entity/Link/Metric 草案 |
| D4 | SoR 绑定 + 冲突表 | bindings；冲突显式化 |
| D5 | Action 分级 + HITL | ≤10 ActionType；权限矩阵 |
| D6–7 | 只读接地 + 金标 | Demo（只读）；eval 基线 |
| D8–9 | 开放 1–3 个写动作（HITL） | 影子/人工确认跑通 |
| D10 | 版本冻结评审 | `v0.1.0`；回灌候选 |

稳态：周会 dig 新 CQ/失败金标/漂移/VQ；变更走 Steward→Owner 门禁；双周回灌产品/模板。

#### 4.10.2 七个硬控制点（C1–C7）

| # | 控制点 | 坏结果 | 检查方式 |
|---|--------|--------|----------|
| **C1 Ownership** | 无人可批口径 | 聊天定生死 | Owner 可呼叫；季度仍应答 |
| **C2 Write-back allowlist** | 任意写 | 生产事故 | 工具表 ⊆ ActionType；CI 校验 |
| **C3 Metric SSOT** | 人机双数 | 失信 | 一族指标一个 A；禁止工具内私改公式 |
| **C4 PII / scope** | 串户/泄密 | 合规失败 | 租户测例；投影测例 |
| **C5 Demo ≠ Prod ontology** | 假绑定上线 | 事故 | 强制标签；Promote 检查表 |
| **C6 Drift detection** | 静默分叉 | 不可解释 | 扫 Agent/MCP 是否脱离本体哈希；周报 |
| **C7 Eval↔version** | 改了不知坏 | 回归失踪 | 发布流水线绑 eval |

Intake→Promote 端到端检查表见深潜包 [`daily-ops-and-control-points.md`](../../research/ontology-enterprise-agent-deep-2026-10-15/daily-ops-and-control-points.md) §3。

### 4.11 反模式（与 F1–F10 / 操作面合并去重）

| 反模式 | 对应 | 纠正 |
|--------|------|------|
| 先上 Agent 再补口径 | F6 近亲 | CQ → Lite → 再暴露工具 |
| 聊天记录 / Confluence 当本体（无绑定） | 操作面 | 无 Binding 则 Agent 仍啃生表 |
| Big-bang 一年建全球图谱 | 操作面 | 按场景切片 + v0 闸门 |
| 只有实体没有动作 | **F2** | L2 Action + HITL |
| 有动作无权限/审计 | **F3** | submission criteria + 审计 |
| Agent 读裸表 / 直接 SQL | **F1** | 默认拒；对象/语义层 |
| 每 Agent 私藏 MCP / 第二套 ARR | **F4** | 共享世界 + 投影 |
| Demo 本体当 Prod | **F8** | 标签 + Promote |
| 无版本 / 变更走聊天 | **F9** | semver + escalation pack |
| 客户域与 FDE 工作知识混库 | **F10** | 分 namespace |
| BI/工具偷 A | RACI | Owner=A；Custodian 不偷 A |
| 用 VQ 补烂模型 | **F6** | 先修 description/关系/指标 |
| FDE 知识不回灌 | 作业模型 | 回灌队列进产品 |

---

## 5. 工作台能力

**结论：** 工作台是 **控制面**；coding agents 是执行器；OpenClaw 是可选通道；对话面只做草稿。在 CodeG Option A 已有锚点上做 **FDE 轨道增量**，不换 Cordis/DSH。**工具栈修订不改变五条产品赌注。**

控制面必须同时覆盖：**本体日常控制面**（CQ / Lite / 绑定 / 投影 / eval / Demo·Prod / 漂移，详下与 §4）与 **履约控制面**（Project / attempt / HITL / 证据 / 回灌 / 环境剖面）。

### 5.1 FDE 活动 → 工作台 → CodeG 映射（wrap 清单）

| FDE 活动 | 工作台必须 wrap | CodeG Option A 锚点 | FDE 轨道增量 |
|----------|----------------|---------------------|--------------|
| 多客户并行 | **项目一等公民**；合规级别；WIP 可见 | Project 容器 | 干系人、成功指标、承诺 vs SOW |
| 发现与访谈 | 笔记、系统地图、纪要；**CQ 模板** | 资料区 | 发现模板与风险清单 |
| 本体建模 | **Lite 编辑 + 版本 diff** | （缺口） | 实体/链接/指标/动作 + semver |
| 绑定与新鲜度 | **SoR / Binding 浏览器** | 连接器契约 | 权威源、冲突表 |
| 暴露给 Agent | **投影白名单 + snapshot 注入** | runner / attempt | objects/actions 子集；`ontology_version` |
| Demo / 环境 | Demo Kit、**Demo/Prod + 剖面 L0–L3 标签** | Automation / 会话 | Promote 门禁；拒绝剖面不允许的工具 |
| 编码实现 | **多 Agent 运行器** + attempt 隔离 | 双运行时、worktree | Claude Code / Codex / Grok Build 适配器 |
| 权限与危险操作 | HITL 三分 | HITL 提问/权限/验收 | **ActionType 级**审批 |
| 证据与验收 | Preview、版本、验证依据 | 成果与证据 | **Eval 跑批**、金标集 |
| 漂移与治理 | **漂移告警**（MCP/工具哈希 vs 本体） | 观测 | 周报；C6 |
| 交办与常驻 | 通道入站、定时 | 频道入站 | **OpenClaw** 可选（allowlist） |
| 回灌产品 | 回灌队列 | （弱） | Playbook / 产品化候选看板 |
| 书面轨迹 | 阻塞纪要 / 决策记录模板 | 资料区 | 可先 Markdown，后对象化 |
| 审计 | 会话与动作日志 | 会话日志 | 客户安全评审可导出 |

> 原则：**工具是执行器；工作台是控制面。** 不把 Chat 当唯一 UI。固定壳优先——危险 HITL 不用开放 HTML 生成 UI。

### 5.2 本体日常控制面（与 §4 对齐；不换栈）

| 控制面能力 | 优先级 | 对应控制点/红线 | CodeG 增量落点 |
|------------|--------|-----------------|----------------|
| **CQ / 工作坊模板** | **P0** | C1；v0 闸门 | Project 资料区模板 |
| **Ontology Lite 编辑 + 版本 diff + changelog** | **P0** | C7；F9 | **增量模块**（域包） |
| **Binding / SoR 浏览器** | **P0** | C3 | 连接器契约视图 |
| **投影 / 动作白名单**（工具表 ⊆ ActionType） | **P0** | C2；F1/F4 | runner 策略 + CI |
| **ontology snapshot 注入**（含 `ontology_version`） | **P0** | C7；§4.8 | attempt 启动上下文 |
| **Demo vs Prod 本体标签** + Promote | **P0** | C5；F8 | 环境/域包元数据 |
| **Eval 跑批**（金标 + 越权轨迹应失败） | **P1**（无则不可 Promote） | C7；F6 | 证据包 / 回归门 |
| **漂移告警** | **P1** | C6；F4 | 观测与周报 |
| ActionType ↔ HITL 三分轨 | **P0** | C2；F3 | HITL 提问/权限/验收 |
| 回灌候选队列 | **P1** | §3.5 飞轮 | 模板市场 |

**Ontology 工具强制绑定（摘要，详 §4.8 / §6）：** 读=对象查询+ACL；写=仅 ActionType；请求带 `ontology_version`；拒绝裸 SQL/任意 SMTP；每次动作写 Evidence。动能弱的语义层 → 工作台**自补 Action + HITL**。

### 5.3 客户环境剖面 L0–L3

| 剖面 | 网络 | 模型 | 通道 | Eval | 工作台行为 |
|------|------|------|------|------|------------|
| **L0 公网试点** | 出网 OK | 原厂 API | OpenClaw 可选 | Promptfoo + 可选 SaaS | 默认试点 |
| **L1 VPC** | 限域出网 | 私有/网关端点 | 内网 IM 或仅 UI | 本地 Promptfoo | Runner 走批准网关 |
| **L2 空气墙** | 无外网 | 客户内模型 | **禁用公网 Gateway** | 本地 only | **拒绝**剖面不允许的工具 |
| **L3 国产化混布** | 按等保 | 国产+可选外模 | 飞书等 | 本地；报告不出域 | 不声称已过客户等保 |

Project 上记录剖面；适配器必实现剖面门禁。中国路径默认考虑 L2/L3 可达性。

### 5.4 P0 / P1 / P2 总清单（能力 + 工具依赖）

#### P0（无则无法安全履约）

1. Project / Engagement 容器（约束、资料、干系人、成功指标、**环境剖面**、凭据**引用**）  
2. Attempt 级隔离（目录或 worktree）+ 会话日志  
3. HITL 三态队列（提问/权限/验收）绑 `execution_attempt_id`  
4. Artifact + Evidence 存储与「确认采用」语义（≠ merge）  
5. **至少 1 个编码 Agent 适配器**（Claude Code **或** Codex 先通；接口预留第二家）  
6. 项目记忆/策略即代码：`CLAUDE.md` / `AGENTS.md` + 权限 deny 列表（与 git 同版）；**CLAUDE.md 不替代强制策略**  
7. 密钥只引用（本机钥匙串 / 客户 Vault）；审计日志；**无明文密钥进 Git**  
8. 固定壳 UI（**勿**首期开放 HTML 生成 HITL）  
9. **Eval 门**：Promptfoo（或等价本地 harness）+ Git 金标集；「可用」标签前置条件  
10. **本体控制面最小集**（§5.2）：CQ、Lite+版本、Binding、投影/白名单、snapshot、Demo/Prod 标签  
11. 书面轨迹模板（阻塞纪要 / 决策记录）  

#### P1（FDE 差异化）

1. Ontology Lite 完整 UX（冲突表、`context=`）  
2. 第二编码执行器 / **Grok Build ACP**  
3. **OpenClaw**（allowlist + `security audit`）；中国区 SLA/等保**需实测，不背书**  
4. IDE 辅写（Cursor 人审）  
5. 对话起草（Claude / ChatGPT / **Grok 对话**——脱敏；**≠ Grok Build**）  
6. Demo Kit（行业包）  
7. Eval 运行器完整 UX + 漂移告警  
8. 回灌 / Playbook 库 + Linear/Jira 链接  
9. 观测：Phoenix 自托管或 LangSmith/Braintrust（若允许 SaaS）  

#### P2（规模化）

1. 租户级策略模板（managed settings 类比）  
2. 成本/配额仪表盘；多客户 WIP 健康度  
3. OSI/语义导入导出（Metric 可靠拢；**仍需自管 Action**）  
4. 伙伴可复制交付包格式  

详 [`../../research/fde-role-and-ai-tools-deep-2026-10-15/recommended-stack-and-workbench.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/recommended-stack-and-workbench.md)。

### 5.5 与 Option A 五条赌注的对齐（再确认）

| 赌注 | 本方案态度 |
|------|------------|
| 1 项目一等公民 | **保持**；扩展合规/干系人/成功指标/**环境剖面**；域包挂项目 |
| 2 默认 Delegate | **保持**；多 runner 由主助手合成；子 Agent 仅在投影内行动 |
| 3 HITL 分轨绑 attempt | **保持**；叠加 ActionType 审批；沙箱 ≠ 验收 |
| 4 attempt 隔离 | **保持**；snapshot 入 attempt；并行 worktree |
| 5 固定壳先于开放 UI | **保持**；Demo/HITL 不走开放 HTML |

**工具栈修订不改变上述五条赌注；不换 Cordis/DSH。**

---

## 6. Agent 工具矩阵

**结论：** **分层，不要混层。** 控制面（CodeG） / 编码执行器（Claude Code · Codex · Grok Build） / 通道（OpenClaw） / 对话草稿（**Grok ≠ Grok Build**） / Eval 门。权限必须**叠层**：厂商沙箱/规则 + 工作台 HITL + Ontology Action 白名单；`CLAUDE.md`/prompt **不替代**强制策略。

**分词硬规则：** Grok（对话）≠ Grok Build（编码 Agent）；**OpenClaw ≠ OpenAI**（开源自托管 Gateway，OpenClaw Foundation）。

### 6.1 按 FDE 任务选层

| FDE 任务 | 首选层 | 典型工具 |
|----------|--------|----------|
| Discovery 笔记 / SOW 草稿 / 邮件纪要 | 对话面 | Claude / ChatGPT / **Grok（对话）** |
| 客户仓库深改、MCP、长会话 | 编码执行器 | **Claude Code** |
| 可重复流水线、沙箱策略、CI `exec` | 编码执行器 | **Codex** |
| 控制面嵌入、并行 worktree attempt | 编码执行器 | **Grok Build**（ACP） |
| 人在 IDE 里快速改/审 | IDE | Cursor / Windsurf / Copilot |
| 客户 IM 交办、常驻助理 | 通道 | **OpenClaw** |
| 金标回归 / 红队 | Eval | Promptfoo（本地）+ 可选 Braintrust/LangSmith/Phoenix |
| 对象查询 / 受控写回 | 语义 | Ontology Lite → MCP 工具投影（§4） |

### 6.2 分工具画像（任务适配 × 客户环境 × 失败模式）

| 工具 | FDE 最适合 | 权限 / HITL 要点 | 客户环境注意 | 典型失败模式 |
|------|------------|------------------|--------------|--------------|
| **Claude Code** | monorepo 深改；交付 MCP/skills；`CLAUDE.md` 链本体摘要 | deny→ask→allow；managed settings 优先；**CLAUDE.md 只引导不强制** | VPN 本机强；PII 用 Read deny + sandbox | 误开 `bypassPermissions`；权限写在 prompt 里以为安全 |
| **Codex** | 批修、CI `exec`、沙箱策略清晰时 | `approval_policy` + `sandbox_mode`；MCP 工具级 approval | 空气墙默认**只用本地**；勿默认 Work Cloud | `never`+`danger-full-access`；**沙箱当验收** |
| **Grok Build** | ACP 嵌入 CodeG；`-w` worktree 并行 attempt | 由**工作台**统一 HITL | 需 xAI API 或客户兼容 `base_url` | 与对话 Grok 混称；worktree 未清理致残留 |
| **OpenClaw** | Slack/飞书/Telegram/… 交办；`/approve` | 一 Gateway=一信任域；DM pairing/allowlist；`security audit` | 中国区 SLA/等保**需实测，不背书** | `dmPolicy: open`；开放 DM+全工具；当 IDE 替代 |
| **Grok（对话）** | 调研、叙事草稿、头脑风暴 | 产出落项目工件再交编码 Agent | 默认不上传客户生产数据 | 无审写回；与 Build 混称 |
| **Cursor 等 IDE** | 人审 diff、微调用 | 企业策略以租户为准 | — | 与 CLI 执行器同目录互踩 |

> OpenClaw：openclaw.ai / docs.openclaw.ai；GitHub `openclaw/openclaw`。官方定位：助手与协调层，**不是** IDE 替代——最快编码环用 Claude Code 或 Codex。

### 6.3 推荐拓扑与 Combo H1–H5

```
客户 IM（飞书/Slack/…）──► OpenClaw Gateway（allowlist + /approve）
                              │
                              ▼
                     CodeG 工作台控制面
                     Project · Ontology Lite · HITL · Evidence · Eval 门
                              │ Delegate
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
        Claude Code        Codex         Grok Build
        （深改/MCP）     （沙箱/CI）    （ACP/worktree）
              │               │               │
              └─────────── git attempt ───────┘
对话面 Claude/ChatGPT/Grok：只用于草稿与澄清，不直连生产写回。
```

| Combo | 适用 | 要点 |
|-------|------|------|
| **H1 标准 B 端试点（有 IM）** | 默认可开 | OpenClaw→attempt；Delegate→Claude Code/Codex；写回→HITL；Promptfoo 绿才「可试用」；周五回灌 |
| **H2 空气墙 / 强不出域** | L2/L3 | 禁公有对话默认；本地/Phoenix；OpenClaw 仅内网或不用；sandbox 默认 read-only |
| **H3 并行多 attempt** | 多方案探索 | Grok Build/Claude Code worktree；独立证据；合并仅 PR+验收 HITL |
| **H4 Anthropic 向客户** | 栈亲和 | Claude Code + MCP/skills 为工件；TDL 管 SOW；OpenClaw 可选 |
| **H5 OpenAI 向客户** | 栈亲和 | Codex + eval 反馈环对齐「采用/影响/eval」度量话术；注意数据驻留条款 |

### 6.4 权限叠层（硬纪律）

| 层 | 管什么 | 不替代什么 |
|----|--------|------------|
| **1. 厂商沙箱/规则** | Claude deny/ask/allow；Codex sandbox；OpenClaw allowlist | **≠** 业务验收 |
| **2. 工作台 HITL** | 提问 / 权限 / 验收绑 attempt | 最终业务门 |
| **3. Ontology Action 白名单** | 工具表 ⊆ ActionType；投影子集 | 世界模型真相源（§4） |

`CLAUDE.md` / prompt / 对话约定 **不替代** managed settings、sandbox、HITL、Action 白名单。能力令牌 = manifest ∩ 节点能力 ∩ 用户授权（对齐 Option A）。

### 6.5 Ontology 绑定（工具强制；交叉引用 §4）

| 检查 | 通过标准 |
|------|----------|
| 读路径 | `query` 对象/属性/链接；对象级 ACL；**禁止生表/DLO** |
| 写路径 | 仅注册 **ActionType**；预检/幂等/回滚说明 |
| 权限 | 继承用户/服务身份；禁止默认上帝账号 |
| 版本 | 带 `ontology_version`；eval 同版本 |
| 拒绝 | 裸 SQL、任意 SMTP、任意文件系统（除非沙箱 attempt 目录） |
| 观测 | 每次动作写 Evidence（谁、何时、哪对象、结果） |
| 多 runner | 只换执行器，**不换世界模型**；编队共享同一客户域投影 |

### 6.6 工具侧反模式红线

1. **无界 Agent 写回**（邮件/工单/DB）无动作白名单  
2. **影子 IT**：个人 ChatGPT/Grok 粘贴客户 PII  
3. **无本体绑定**：工具 = 裸 SQL + 裸 HTTP  
4. **沙箱 = 验收**：跳过业务 HITL  
5. **多执行器同目录互踩**  
6. **OpenClaw 开放 DM + 全工具**  
7. **Demo 环境凭据复用生产** / Demo=Prod  
8. **Grok 与 Grok Build 混称**；把 OpenClaw 写成 OpenAI  

详 [`../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-ai-tools-matrix.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/fde-ai-tools-matrix.md)。

---

## 7. Fast Demo + 基线 AI 能力框架

**结论：** Demo 框架必须 **模块化 + 金标 + 门禁**。目标是 48–72 小时（有行业模板时）拉起可演示垂直场景，且同一骨架可升级为试点——不是一次性演示脚本。

### 7.1 框架目标

1. **快**：有模板时 48–72 小时可演示。  
2. **可升级**：换真实连接器、开 HITL 写动作、加观测 → Pilot。  
3. **可沉淀**：行业模板包进 CodeG 模板库 / 数字员工包。  

### 7.2 Must-have 模块表

| # | 模块 | 最低能力 | 生产升级点 | 验收线索 |
|---|------|----------|------------|----------|
| 1 | **身份与租户** | 本地用户/API Key；项目级隔离 | SSO/OIDC、客户 IdP、RBAC | 越权测试失败 |
| 2 | **密钥与配置** | `.env` / 钥匙串引用 | Vault、轮换、审计 | 无明文密钥进 Git |
| 3 | **连接器** | ≥2：文件/网页 + 1 业务 API（只读） | 写回、分页、限流、重试幂等 | 契约测试 |
| 4 | **RAG / KB** | 切块、嵌入、检索、引用展示 | 权限过滤、增量索引、混合检索 | 引用可点回源 |
| 5 | **Ontology Lite** | 实体/关系/指标/动作（YAML/JSON） | SoR 绑定、版本、审批 | Agent 不直接扫裸表 |
| 6 | **工具 / MCP** | 统一注册；允许/拒绝列表 | 客户 MCP、费率、沙箱 | 未授权工具不可见 |
| 7 | **Agent 运行时** | 单主 Agent + 可选 subagent | 多 runner 适配 | attempt 日志完整 |
| 8 | **HITL** | 提问/权限/验收三队列 | SLA、升级干系人 | 无静默写生产 |
| 9 | **Eval** | 金标集 ≥20；离线跑分 | 回归门禁、线上抽样 | 分数可复现 |
| 10 | **观测** | 结构化日志、trace id、token/费用 | OpenTelemetry、告警 | 可按 attempt 回放 |
| 11 | **部署剖面** | Docker Compose 本地 | K8s/私有化/空气间隙 | 一键脚本文档化 |
| 12 | **Demo UI 壳** | 固定壳：聊天+对象+证据 | 客户品牌、权限视图 | HITL 非开放 HTML |
| 13 | **行业模板** | ≥1 个垂直包 | 多行业市场 | 含本体+数据+eval |
| 14 | **成本与配额** | 显示本次 token/费用 | 预算熔断 | Demo 不烧穿账户 |

### 7.3 行业模板包结构

每个 `templates/<industry>/`：

```
ontology.yaml          # 实体/链接/指标/动作
sample_data/           # 脱敏或合成
connectors.mock.yaml   # 模拟 API
golden_eval.jsonl      # 问题/任务/期望
demo_script.md         # 5–8 分钟演示词
threat_model.md        # 数据与动作风险
promotion_checklist.md # Demo→Pilot 门禁
```

| 建议首批模板 | 核心对象 | 黄金任务例 |
|--------------|----------|------------|
| 工单/客服运营 | Ticket, Customer, Policy | 分类、摘要、草拟回复（HITL 发送） |
| 供应链异常 | Order, SKU, Shipment, Plant | 解释延迟、建议改派（只读+建议） |
| 内部知识助手 | Doc, Person, Project | 带引用问答、找 owner |
| 数据分析助手 | Metric, Dashboard, Table | 语义层问答（**禁裸 SQL**） |
| 代码交付助手 | Repo, PR, Attempt | 修 bug、出 PR 说明 |

### 7.4 Demo → Pilot → Production 门禁

| 门禁 | Demo | Pilot | Production |
|------|------|-------|------------|
| 数据 | 合成/脱敏 | 真实只读子集 | 正式权限域 |
| 动作 | 无或假写 | HITL 写 | 策略化写 + 审计 |
| Eval | 手工金标 | 周回归 | 发布门禁 |
| 观测 | 基础日志 | 仪表盘 | on-call runbook |
| 所有权 | FDE | FDE+客户 | 客户主责 / 支持备份 |
| 回灌 | 记录痛点 | 候选特性 | 进产品路线图 |

**红线：** 未过 Pilot 门禁不得宣称「已生产落地」。环境剖面（§5.3）为 L2/L3 时，Eval 仅本地、通道禁用公网 Gateway；**不声称**已过客户等保。


### 7.5 快速交付节奏（参考）

| 日 | 动作 |
|----|------|
| D0 | 选模板；确认成功指标与禁区 |
| D1 | 装基线；接样本；裁本体 |
| D2 | 接只读连接器；跑通 5 黄金任务 |
| D3 | 排练 Demo；补 HITL；写风险与下一步 SOW |

并行：用 Claude Code/Codex 生成连接器样板；用工作台固证据。

### 7.6 Demo 反模式红线

1. 现场现写无模板的一次性 Demo，结束即弃  
2. 无引用、无金标的「效果很好」叙事；**无 eval 门**宣称可用  
3. Demo 环境共用客户生产凭据；**Demo=Prod**  
4. Agent 可任意 SQL / 任意发邮件；**无本体绑定**  
5. 无成本看板导致演示打爆账单  
6. 把 UI 生成壳当 HITL 界面  
7. **剖面 L2 空气墙**下仍默认公网对话 / 公网 OpenClaw（须禁用或仅内网）  
8. **影子 IT**：个人 ChatGPT/Grok 粘贴客户 PII  
9. **沙箱当验收**，跳过业务 HITL  

### 7.7 最小技术素描（非指定实现）

```
gateway/          # API + auth
ontology/         # Lite schema + versions
connectors/       # read/write adapters
rag/              # ingest + retrieve
agents/           # orchestrator + runner adapters
hitl/             # queues
eval/             # golden + runner
obs/              # log/trace/cost
templates/        # industry packs
web/              # fixed shell UI
deploy/           # compose / helm
```

映射 CodeG：`gateway+agents+hitl` ≈ Substrate；`templates` ≈ 数字员工/行业包；`ontology` ≈ 新模块。

---

## 8. 落地路线图（与 CodeG 里程碑对齐）

**结论：** 增量叠在 Option A 的 M0–M6 上；**不换底座**。下表为建议插入点，供排期，非排期承诺。

| 阶段 | CodeG 锚点 | FDE 轨道增量 | 验收线索 |
|------|------------|--------------|----------|
| **F0 契约** | M0 对象与契约 | 冻结 Ontology Lite schema；Engagement/回灌队列对象草图；本方案术语 | schema 可评审 |
| **F1 控制面加固** | M1–M3 协作 API + HITL + 成果 | 客户字段进 Project；ActionType 级权限请求；Eval 报告作为 Evidence 类型 | HITL 可挂动作审批 |
| **F2 单 runner** | M2–M5 | 接入 Claude Code **或** Codex 之一；attempt 日志完整 | 客户仓可跑通一条集成任务 |
| **F3 Ontology Lite MVP** | 新模块（并行 M2+） | 浏览器/编辑器；YAML 导入；snapshot 注入 runner | Agent 默认不见裸表 |
| **F4 Demo Kit v0** | 模板市场雏形 | ≥1 行业模板（本体+假数据+金标+演示词+门禁清单） | 72h 内可演示 |
| **F5 多 runner + 通道** | M4 触发器 / 频道 | Codex/Grok Build 适配器；OpenClaw **可选**接入 | 决策表可执行 |
| **F6 回灌与规模化** | M6 UI 扩展预留 | Playbook 库；产品化候选看板；成本仪表盘；第二行业模板 | 回灌条目可进路线图 |

| 明确不做（与 Option A 一致） | 理由 |
|------------------------------|------|
| 换 Cordis/DSH 底座 | Option A 已否决首期换栈 |
| 首期承诺 Foundry 级全量本体 | Ontology Lite 边界 |
| 首期组织级 SSO/治理为主轴 | 个人 FDE 台优先；客户 IdP 按项目接入 |
| 开放 HTML 作 HITL/验收 | 越权风险 |
| 未实测即承诺 OpenClaw 等保 | 合规需实测 |

---

## 9. 附录

### 9.1 美中 FDE 表摘要

见 §2.2 / §2.3；完整矩阵：[`../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md`](../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md)。

### 9.2 不确定性与软写项（诚实标注）

| 项 | 状态 / 写法 |
|----|-------------|
| **Databricks FDE 岗位 URL** | 2026-10-15 fetch **404**；职责用「曾公开招聘快照」语气 |
| **腾讯等深核案例** | research 包缺口；**勿编造** |
| **AtScale Text-to-SQL 20%→90%** | Snowflake 博客引用的**第三方基准**，非本包复现 |
| **Anthropic 周薪等自媒体数字** | **不写入**正文 |
| **「中国全面 FDE 化」** | 实际是**职责同构、名称异构** |
| **OpenClaw 企业合规 / 中国通道** | 需实测；**不承诺**等保一次过 |
| **Anthropic Greenhouse 深链** | 搜索摘要含完整 JD；直接 fetch 曾不稳——正式引用以多源交叉为准 |
| **商汤案例 ROI** | 媒体报道可引场景；**勿夸大未证实 ROI** |
| **火山/百度具名客户深度** | 产品能力材料多、独立深核案例少 → 软写 |
| **Grok Build vs 对话 Grok** | 已分清；全文禁止混称 |
| **与 Foundry 功能一对一** | **禁止**；只写机制对齐与 Lite 子集 |
| **Salesforce P50&lt;200ms / AtScale 准确率跃迁** | **他方叙述**；可引作公开说法，不可写成我方复现 ROI |
| **Fabric Ontology GA** | 实为 **Preview**；旧体验退役 **2027-01-31**；写可参考模式 |
| **蚂蚁/阿里图谱细节** | 公开分享与二手转述并存；**中等置信**；勿当我方一手审计流程 |
| **「买了图谱=有运营本体」** | **禁止**；中国路径常需自补动作层（冲 L2） |
| **未授权具名客户 ROI** | 一律软写或不写 |
| **Staffing 比率 / 内部计费** | **未公开**；勿写成确定 KPI |
| **OpenClaw 等保 / 中国区 SLA** | **未替客户实测**；不背书 |
| **面试周数 / 差旅均值** | 从业者报道近似（B）；勿当雇主官方 SLA |
| **Grok vs Grok Build / OpenClaw≠OpenAI** | 全文硬分词；禁止混称 |
| **Scale「数百小时/周」样例** | 雇主 JD 样例叙事（A 来源、**非独立审计**）；可作「替人干完活」话术，**勿当合同 KPI** |
| **品玩商汤案例 ROI%** | 媒体 **C**；可引场景，勿硬写成已审计事实 |
| **媒体/自媒体 ROI%** | **禁止**写入硬承诺或合同级验收 |
| **Palantir Delta vs Dev** | 官方博文要点可软写；本轮部分 CF 未读全文 → **B / A−待核** |
| **阿里云 MaaS SA 职责全文** | careers SPA；本包取自搜索高亮（A−）；已写明**协同 FDE**、SA≠FDE |
| **「FDE trap」** | LinkedIn/从业者讨论（B/C）；反模式软写 |

### 9.3 来源回溯

全部 URL 与访问日见：

- FDE / 浅层 Ontology：[`../../research/fde-ontology-b2b-ai-2026-10-15/sources.md`](../../research/fde-ontology-b2b-ai-2026-10-15/sources.md)
- Ontology 深潜：[`../../research/ontology-enterprise-agent-deep-2026-10-15/sources.md`](../../research/ontology-enterprise-agent-deep-2026-10-15/sources.md)
- FDE 角色+工具：[`../../research/fde-role-and-ai-tools-deep-2026-10-15/sources.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/sources.md)
- FDE 定义/价值/度量三角：[`../../research/fde-definition-value-deep-2026-10-15/sources.md`](../../research/fde-definition-value-deep-2026-10-15/sources.md)


关键一手入口（摘要，细节以 sources 为准）：

| 类别 | 代表来源 |
|------|----------|
| 美 FDE JD | Palantir Lever；OpenAI careers FDE；Anthropic Greenhouse FDE/TDL；Scale Accel jobs；Databricks careers（404 已标） |
| 从业者 | fdeinterviews.com（日常/对照/PoC→Prod） |
| 中国 | 商汤猎聘「FDE 工程师」（A）；品玩商汤报道（C，ROI 软写）；第四范式建设服务页；华为 iDME；阿里 MaaS SA（协同 FDE，A−） |
| Ontology | palantir.com/docs Ontology；Salesforce Data 360 语义层博客；Snowflake Context Layer 博客；Microsoft Learn Fabric Ontology；华为云 iDME |
| Agent 工具 | code.claude.com；developers.openai.com/codex；docs.x.ai/build；openclaw.ai / docs.openclaw.ai |
| 上游 CodeG | [`../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md`](../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md) |

### 9.4 本包文件与 research 索引

| 文件 | 用途 |
|------|------|
| 本 `proposal.md` | 正式方案正文 |
| `README.md` | 包说明与读者入口 |
| [`../../research/fde-ontology-b2b-ai-2026-10-15/research-brief.md`](../../research/fde-ontology-b2b-ai-2026-10-15/research-brief.md) | FDE 美中矩阵调研综合 |
| [`../../research/fde-ontology-b2b-ai-2026-10-15/handoff-to-方案撰写.md`](../../research/fde-ontology-b2b-ai-2026-10-15/handoff-to-方案撰写.md) | 初版撰写交接 |
| [`../../research/ontology-enterprise-agent-deep-2026-10-15/research-brief.md`](../../research/ontology-enterprise-agent-deep-2026-10-15/research-brief.md) | Ontology 深潜综合（建设/抽取/Agent/日常） |
| [`../../research/ontology-enterprise-agent-deep-2026-10-15/handoff-to-方案撰写.md`](../../research/ontology-enterprise-agent-deep-2026-10-15/handoff-to-方案撰写.md) | §4 加深交接 |
| [`../../research/fde-role-and-ai-tools-deep-2026-10-15/research-brief.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/research-brief.md) | FDE 角色+工具栈加深综合 |
| [`../../research/fde-role-and-ai-tools-deep-2026-10-15/handoff-to-方案撰写.md`](../../research/fde-role-and-ai-tools-deep-2026-10-15/handoff-to-方案撰写.md) | §3/§5/§6 加深交接 |
| [`../../research/fde-definition-value-deep-2026-10-15/research-brief.md`](../../research/fde-definition-value-deep-2026-10-15/research-brief.md) | FDE 定义/价值/度量三角加深综合 |
| [`../../research/fde-definition-value-deep-2026-10-15/handoff-to-方案撰写.md`](../../research/fde-definition-value-deep-2026-10-15/handoff-to-方案撰写.md) | **§2/§3** 定义·价值加厚交接 |

### 9.5 说明

- 本文不编造星数、客户 ROI、未访问 URL。  
- 未改写 CodeG Option A 正文，未改 research 原文；§4 按深潜包加深；§2/§3 按定义·价值包加厚，**不推翻** Option A；未改 §3.3 日课 / §6 工具矩阵 / Ontology 深潜专节结构。  
- 「可映射 / 可落地」指方案层接线，**不**表示作者已跑通端到端实现。  

---

*文档结束。*
