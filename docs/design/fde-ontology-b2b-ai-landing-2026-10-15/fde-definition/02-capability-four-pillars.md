# 02 — 四象限能力 + 美中具名对照

| 字段 | 内容 |
|------|------|
| **结论先行** | FDE = **懂技术 + 懂业务 + 能开发 + 能交付**（四象限缺一不可）；对内统一称 FDE，对外可双语；**SA 默认 ≠ FDE** |
| **对应总册** | [`../proposal.md`](../proposal.md) **§2.1.1**（四象限）、**§2.2–§2.5**（美中具名） |
| **上游 research** | [`../../../research/fde-definition-value-deep-2026-10-15/02-capability-profile.md`](../../../research/fde-definition-value-deep-2026-10-15/02-capability-profile.md) |
| **不写** | 面试环长表、Great vs Average 全表（见 proposal §2.6 / §3.4） |

---

## 四象限 → 可观察行为（硬）

| 象限 | 现场能看到 | 缺一则症状 |
|------|------------|------------|
| **懂技术** | 客户边界内架构取舍：SoR、鉴权、egress、数据驻留；选 RAG vs Agent vs 规则 | 方案无法落地；安全/IT 卡死 |
| **懂业务** | 模糊痛点钉成 **1 条可测工作流** + 成功数字 + 非目标 | 漂亮但无人用的 Demo；KPI 对不上赞助人 |
| **能开发** | **亲手**写/审生产代码；交付连接器、Agent、MCP/tools、**写回路径** | 变成售前/顾问；所有权断裂 |
| **能交付** | eval 门 + launch criteria；影子→正式；培训/handoff；**回灌条目**写出 | 笔记本 PoC 永不 Harden；无采用 |

> 形容词不够；招聘与复盘都筛「现场能看到的行为」。

---

## 美国：具名短表（硬）

| 公司 | 公开职衔 | 要点 |
|------|----------|------|
| **Palantir** | FDSE / Forward Deployed AI Engineer；内部常称 **Delta** | 嵌入客户；定制应用 + LLM 工作流；outcome 所有权 |
| **OpenAI** | Forward Deployed Engineer（含 Healthcare） | own discovery→**production rollout**→adoption/handoff；度量三角最齐 |
| **Anthropic** | FDE（Applied AI）+ **Technical Deployment Lead** | FDE **写生产码**（含 MCP/sub-agents/skills）；**TDL 明文不写码** |
| **Scale AI** | FDAIE / **Frontier Agents Engineer**；**AI Deployment Strategist** | 客户边界内集成 + eval harness；Strategist 管 SLA/扩张 |
| **Databricks** | Forward Deployed Engineer（PS） | 数据+AI 生产化（岗位页曾 404；以曾公开快照为准 — **软**） |

详 URL：research `sources.md`；矩阵：[`../../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md`](../../../research/fde-ontology-b2b-ai-2026-10-15/fde-us-cn-matrix.md)。

---

## 中国：具名短表（硬 / 软分标）

| 组织 | 公开职衔或服务名 | 与美式 FDE | 来源级 |
|------|------------------|------------|--------|
| **商汤科技** | **FDE 工程师**（猎聘官方 HR） | **同构品牌**：明确用 FDE | **硬**（A） |
| **第四范式** | **AI 建设服务**（含投产） | 职责同构、名称异构（服务菜单） | **硬**（A 官网） |
| **阿里云** | **MaaS 解决方案架构师（政企）**；JD 写**协同 FDE 团队** | **异构偏 SA** + 显式与 FDE 分工 | **硬–中**（A− SPA） |
| 火山 / 百度 / 腾讯等 | 方案架构 / 交付 | 名称异构；公开 FDE 品牌弱 | **软**（本包未深核） |

媒体案例 ROI%（如品玩）：可引场景，**勿当合同 KPI**（**软 / C**）。

---

## 美中对照 → 对 Mack 的含义（硬）

| 维度 | 美国 | 中国 | 含义 |
|------|------|------|------|
| 品牌头衔 | 直接叫 FDE；Palantir 内部 Delta | 商汤叫 FDE；多数叫交付/驻场/SA | **对内统一 FDE，对外可双语** |
| 写生产码 | JD 高占比；FDE 写 / TDL 不写 | 商汤 FDE、范式建设上线；阿里 SA 偏方案+协同 FDE | **SA ≠ FDE**，除非含生产码与上线所有权 |
| 配对 | TDL / Strategist | 售前 SA + 交付/FDE | 可抄 Anthropic 拆分 |
| 回灌 | Codify / patterns（JD 义务） | 商汤闭环；范式偏项目；阿里反馈产品 | **回灌强度是分水岭** |
| 度量 | adoption / impact / eval | 验收、见效、续费 | 主度量用 OpenAI 三角（见 03） |

---

## 对外话术 → 对内统一（硬）

| 对外（CN） | 对内 | 风险 |
|------------|------|------|
| 前线部署工程师 / FDE | **FDE** | — |
| 驻场工程师 | FDE（驻场履约） | 可能弱化回灌 |
| 交付工程师 | FDE（履约阶段） | **易停在人天** → 须显式回灌队列 |
| 解决方案架构师 | SA；可与 FDE **配对** | 偏售前时**不要等同** FDE |
| 建设服务（第四范式） | FDE 能力外包/原厂服务包 | 服务菜单 ≠ 个人职衔 |

---

*下一篇：[`03-value-get-work-done.md`](03-value-get-work-done.md)*
