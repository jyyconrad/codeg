# FDE × Ontology Lite × B 端 AI 落地 — 方案包

| 字段 | 内容 |
|------|------|
| **日期** | 2026-10-15（Asia/Manila / UTC+8） |
| **状态** | 正式方案索引（正文见 `proposal.md`） |
| **性质** | **新轨道**：FDE 运营层 + Ontology Lite + Demo/基线框架；**互补** CodeG，**不推倒** Option A |
| **用户** | Mack |

---

## 读者入口

| 读者 | 用途 |
|------|------|
| Mack | 确认与 CodeG 关系、六条论点、红线与软写项；拍板里程碑插入点 |
| 产品 / FDE 负责人 | 按 §3 作业模型与 §7 Demo 门禁拆交付节奏 |
| 工程 | 按 §5 P0/P1、§6 Agent 矩阵、§8 路线图拆任务 |

**阅读顺序：**

1. 本 README（定位与 CodeG 关系）  
2. **定义/价值（建议先拆读）：** [`fde-definition/04-one-pager.md`](fde-definition/04-one-pager.md) → [`01`](fde-definition/01-what-is-fde.md)–[`03`](fde-definition/03-value-get-work-done.md) → 再回 [`proposal.md`](proposal.md) **§2 / §3**；**内线业务 / 四类产品能力见 [`06`](fde-definition/06-internal-business-four-capabilities.md)**  
3. [`proposal.md`](proposal.md) — 正式方案正文（§0–§9；Ontology/工具/Demo 仍读总册）  
4. 需要证据细节时回 research 包（见下）；**勿**在方案里复制会过时的星数/案例长文  

---

## 与 CodeG Option A 的关系（必读）

| 项 | 说明 |
|----|------|
| **不变** | [`../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md`](../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md) — Option A（调度底座 × 数字员工 × 项目编队）；**五条产品赌注全部保持** |
| **本包角色** | FDE 作业手册 + Ontology Lite + 工作台增量能力 + Demo 框架 |
| **映射** | 可落为 CodeG 的项目/HITL/证据扩展与模板市场；多 coding-agent 适配器；**不要求换栈** |
| **禁止** | 推倒 Option A；把本包写成 Cordis/DSH 换栈理由 |
| **工具栈** | 修订加厚执行器/通道/eval；**不改变**五条产品赌注 |

文首完整声明见 `proposal.md` §0。

---

## 一句话立场

B 端 AI 落地瓶颈是「模型 → 可治理业务动作」。用 **FDE（生产所有权 + 产品回灌 + 动作闭环/write-back）** 跑客户 engagement——买方买的是「替人把活干完」，不是「会答题」；用 **Ontology Lite** 做人机共用语义与动作白名单；用 **工作台控制面 + coding agents 执行器 + 可选 OpenClaw 通道** 编排；用 **模块化 Demo + 金标 + 门禁** 从 PoV 升到生产——全部增量叠在 CodeG Option A 上。

---

## 文件索引

| 文件 | 内容 |
|------|------|
| [`proposal.md`](proposal.md) | **主交付**：正式方案正文 |
| [`README.md`](README.md) | 本说明 |
| [`fde-definition/`](fde-definition/) | **定义/价值拆读册**（领导可读短文）：先读 [`04-one-pager.md`](fde-definition/04-one-pager.md)，再 01–03 / 可选 05-FAQ；**部门内线四类能力见 [`06`](fde-definition/06-internal-business-four-capabilities.md)**；与 proposal §2/§3 对应，不另立政策 |

### 上游 research（只读；四包并列；方案内相对路径引用）

| 包 | 路径 | 用途分工 |
|----|------|----------|
| **初包 · FDE 美中** | [`../../research/fde-ontology-b2b-ai-2026-10-15/`](../../research/fde-ontology-b2b-ai-2026-10-15/) | FDE 美中矩阵、浅层 Ontology、工作台/Demo 初稿；handoff / research-brief / fde-us-cn-matrix / ontology-practices / workbench-and-agent-stack / demo-ai-framework / sources |
| **Ontology 深潜** | [`../../research/ontology-enterprise-agent-deep-2026-10-15/`](../../research/ontology-enterprise-agent-deep-2026-10-15/) | 建设方法、四源抽取、Agent 机制、日常七控制点、L0–L3；专管正式方案 **§4** 加深 |
| **角色+工具加深** | [`../../research/fde-role-and-ai-tools-deep-2026-10-15/`](../../research/fde-role-and-ai-tools-deep-2026-10-15/) | 日/周节奏、技能矩阵、工具分层与 Combo H1–H5、环境剖面 L0–L3；专管 **§3 日课 / §5 / §6** 加深 |
| **定义/价值加深** | [`../../research/fde-definition-value-deep-2026-10-15/`](../../research/fde-definition-value-deep-2026-10-15/) | 作业定义、四象限能力、度量三角、write-back/验收门、TDL 边界；专管 **§2–§3** 加厚 |

### 相邻 CodeG（只读对齐；本包未改写）

| 路径 | 角色 |
|------|------|
| [`../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md`](../codeg-agent-substrate-digital-employees-2026-10-14/proposal.md) | Option A 主方案 |
| [`../codeg-agent-substrate-digital-employees-2026-10-14/README.md`](../codeg-agent-substrate-digital-employees-2026-10-14/README.md) | CodeG 方案包索引 |

---

## 正文各章一句话

| 章 | 摘要 |
|----|------|
| §0 | 文首声明：不推翻 Option A；六条综合论点索引 |
| §1 | B 端鸿沟：模型≠业务动作；FDE+本体为何现在必要 |
| §2 | FDE 定义（全程负责+回灌）；四象限；美中具名（含 Delta/TDL/猎聘 FDE/阿里协同 FDE）；招聘信号（软） |
| §3 | 作业模型：度量三角 + write-back + G1–G7；TDL 不写码；日/周节奏（未改坏）；WIP/红线 |
| §4 | Ontology 加深：建设/抽取/Agent/七控制点/L0–L3（深潜包） |
| §5 | 控制面 wrap + 本体控制面 + 环境剖面 L0–L3；P0/P1 工具依赖 |
| §6 | Agent 矩阵重写：任务选层、权限叠层、Ontology 绑定、Combo H1–H5 |
| §7 | Demo 模块表、行业模板、门禁；L2 空气墙/影子 IT/eval 红线 |
| §8 | 与 CodeG 里程碑对齐的增量路线图 |
| §9 | 来源、美中摘要、不确定性软写项 |

---

## DoD 自检

- [x] 独立正式方案，中文为主  
- [x] 美中 FDE 均有具名公司与角色  
- [x] Ontology 定义区分清晰 + Ontology Lite 范围  
- [x] 工作台 P0/P1 与 Agent 组合决策表  
- [x] Demo 模块清单 + 门禁  
- [x] 文首声明不推翻 CodeG Option A  
- [x] 不确定项有标注；来源可回溯至 research/sources  
- [x] Ontology §4 加深（建设/抽取/Agent/日常控制点）+ 深潜包链接  
- [x] §3/§5/§6 角色+工具加深；research 包并列  
- [x] 工具栈修订不改变 Option A 五条赌注；Grok≠Build；OpenClaw≠OpenAI  
- [x] §2/§3 定义·价值加厚：全程负责、四会、替人干完活、write-back、度量三角、TDL 不写码  
- [x] 四包并列含 `fde-definition-value-deep-2026-10-15`；软写项标来源级；§3.3/§6/§4 结构未改坏  
- [x] `fde-definition/` 拆读册（00–05）：一页纸→01–03→proposal §2/§3；未推倒 Option A  
- [x] `fde-definition/06`：部门内线四类产品能力（智能客服/知识库/语音/AI Coding）；双轨指针入 proposal §2/§3  

---

*包说明结束。*
