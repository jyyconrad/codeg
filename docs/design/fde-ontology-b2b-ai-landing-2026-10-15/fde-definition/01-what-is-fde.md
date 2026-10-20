# 01 — FDE 是什么（作业定义）

| 字段 | 内容 |
|------|------|
| **结论先行** | FDE = **嵌入客户、从 discovery → production rollout / adoption·handoff 全程负责，写生产代码，并把现场模式沉淀成产品能力（回灌）** |
| **对应总册** | [`../proposal.md`](../proposal.md) **§2**（定义与美中）、**§3.1–§3.2**（角色三角与生命周期） |
| **上游 research** | [`../../../research/fde-definition-value-deep-2026-10-15/01-job-definition.md`](../../../research/fde-definition-value-deep-2026-10-15/01-job-definition.md) |
| **不写** | 日课表、工具矩阵、Ontology 长文（见 proposal §3.3 / §5–§6 / §4） |

---

## 一句话（硬）

> 售前卖「可能」；咨询卖「建议与人天」；FDE 卖「**生产里真的在干活**」，并有义务把现场模式**产品化回灌**。

买方买的不是「会答题」，是「**替人把活干完**」——详见 [`03-value-get-work-done.md`](03-value-get-work-done.md)。

---

## 所有权弧（硬）

对齐 OpenAI careers 原文结构：

| 阶段 | FDE 拥有什么 | 「完成」判据 |
|------|--------------|--------------|
| Discovery | 问题发现、约束、系统/数据地图 | 赞助人认可「真正值钱且可测」的问题 |
| Scope / SOW | MVP 边界、成功数字、非目标 | 可签字；非目标写清 |
| Design → Build | 架构、**生产代码**、集成（含 MCP·Agent·写回） | 客户边界内可跑真实/脱敏流量 |
| Production rollout | Harden、eval、上线 | launch criteria 齐备 |
| Adoption / handoff | 培训、影子→正式、移交 | 无原厂盯屏可运行 |
| Productize | Codify playbooks / 模板 / 回灌条目 | **≥1 回灌候选**/里程碑 |

详表见 proposal §3.2。

---

## Own / Not own（硬）

| 维度 | FDE **拥有** | FDE **通常不单独拥有** |
|------|--------------|------------------------|
| 问题与范围 | Discovery、workflow scoping、取舍 | 商业合同条款、定价 |
| 技术解 | 架构、生产代码、集成、eval、上线 | 纯售前赢单叙事 |
| 采用 | guide adoption、handoff | 长期 CS 扩张（可交接） |
| 产品飞轮 | 模式 → playbook / 平台反馈 | 产品路线图最终拍板 |
| 价值度量 | 与 TDL/Strategist **共有**或由其主导 ROI 前后测 | 单独给董事会的 KPI 报表 |

---

## ≠ 售前 ≠ 纯咨询（硬）

| 对照 | 生命周期 | 写生产码 | 成功度量 | 产品回灌 |
|------|----------|----------|----------|----------|
| **FDE** | 售后→**生产** | **高** | 采用 + 工作流影响 + eval | **JD 明文义务** |
| **Solutions / SE** | 售前→早期售后 | 低–中（PoC） | 赢单、方案认可 | 弱 |
| **纯咨询 / 人天** | 项目制账单 | 可变；常一次定制 | 验收与人天 | 弱或无（不进原厂飞轮） |

**反模式（硬）：** 长期驻场但零回灌条目 = **改牌的咨询外包**。

---

## 与 TDL 的硬边界（硬）

Anthropic **Technical Deployment Lead** JD 明文：

| 角色 | 职责 | 写生产码？ |
|------|------|------------|
| **FDE** | 建造技术解：生产应用、MCP / sub-agents / skills、eval、上线、回灌 | **写** |
| **TDL / Strategist** | SOW→production 范围、干系人、**ROI 前后测**、变更管理 | **不写**（alongside FDEs who build） |

```
售前 SE / SA ──赢单/PoC──► │ 签约后
                           ▼
              ┌── TDL / Strategist ── SOW、干系人、ROI（不写生产码）
              └── FDE               ── 生产代码、集成、eval、上线、回灌
                           ▼
                    CS ── 上线后健康度与扩张（可交接）
```

组织默认：一人公司可兼 FDE + 轻量 TDL；**并行客户 ≥2** 应拆「写码」与「范围/ROI」看板。详见 proposal §3.1。

---

## 回灌是义务，不是加分项（硬）

| 机制 | 雇主措辞要点 |
|------|--------------|
| Codify playbooks / building blocks | OpenAI |
| Repeatable patterns → Product/Eng | Anthropic FDE；TDL「field signals」 |
| Customer needs → product improvements | Scale |
| 能力沉淀 / 手册规范 | 商汤叙事（对外可说「沉淀」，对内对齐「回灌」）— 媒体源 **软** |

工作台要有**回灌队列**；无回灌不做长期驻场（见 [`03-value-get-work-done.md`](03-value-get-work-done.md) 红线）。

---

## 软写提醒

| 项 | 标注 |
|----|------|
| 中国「驻场/交付」与美式 FDE **职责同构、名称异构** | **软**（见 [`02-capability-four-pillars.md`](02-capability-four-pillars.md)） |
| Palantir「radical commitment to the outcome」文化表述 | **软**（勿当量化 SLA） |
| 差旅%/Staffing 比率 | **软**（公开区间方差大；见 proposal §3.5） |

---

*下一篇：[`02-capability-four-pillars.md`](02-capability-four-pillars.md)*
