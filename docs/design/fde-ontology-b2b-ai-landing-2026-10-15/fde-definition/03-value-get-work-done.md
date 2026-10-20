# 03 — 价值主张：替人把活干完

| 字段 | 内容 |
|------|------|
| **结论先行** | 企业买的不是「AI 会答题」，而是「**AI 替人把活干完**」：生产采用 + 可度量工作流影响 + 可验收动作闭环（write-back）与 eval 门 |
| **对应总册** | [`../proposal.md`](../proposal.md) **§3**（作业模型）、尤其 **§3.5**（度量三角 / G1–G7）、**§3.7**（红线） |
| **上游 research** | [`../../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md`](../../../research/fde-definition-value-deep-2026-10-15/03-value-proposition.md) |
| **不写** | 日课节奏、WIP 细则（见 proposal §3.3 / §3.6） |

---

## 买方价值句（硬）

> 买方付费对象是 **生产里被采用的、可度量的工作流改变**（含写回/动作闭环与 eval 门），不是聊天窗口答对率。

**操作化：** 采用 ∧ 工作流指标 ∧ 写回/动作闭环 —— **缺一则未交货**。

---

## 度量三角（硬 · OpenAI 原文结构）

| 角 | 定义 | 不是 |
|----|------|------|
| **Production adoption** | 目标用户群在生产环境按设计频率使用 | ≠ 试点账号点开 Demo；≠ 账单人天 |
| **Measurable workflow impact** | 事先约定 KPI：基线 → 后测（时延、吞吐、人工小时、差错率、完成率等） | ≠ 「感觉更快了」 |
| **Eval-driven feedback** | 金标/客户特定 benchmark 失败案例进产品与模型路线图 | ≠ 只修客户侧工单 |

对齐 Anthropic TDL：**pre / post ROI** 报表格式给赞助人。

---

## Write-back：业务轨必要条件（硬）

| 轨 | 度量 | 说明 |
|----|------|------|
| **业务** | 采用 + 工作流 KPI + **动作闭环/write-back**（经治理写入 SoR/工单/审批流；幂等·权限·审计） | **缺写回 = 仍是「会答题的 Demo」** |
| **工程/AI** | 金标 eval、回归门、成本/延迟、HITL SLA、launch readiness | 无金标不得宣称「已生产落地」 |
| **产品飞轮** | 回灌条目 ≥1/里程碑；模板复用次数 | 区分「人天项目」与「FDE」 |

Ontology Lite / 动作白名单提供「可允许的做事方式」——详见 proposal **§4**（本夹不展开）。

---

## 验收门 G1–G7（硬 · DoD）

| 门 | 定义 | 对齐 |
|----|------|------|
| **G1 Production adopted** | 生产按设计频率使用 | OpenAI adoption |
| **G2 Workflow metric** | 基线+后测的工作流 KPI | OpenAI impact；TDL pre/post |
| **G3 Eval gate** | 金标达 acceptance threshold；失败可追溯 | Healthcare；Scale golden |
| **G4 Action / write-back** | 经治理写入 SoR/工单；幂等·权限·审计 | Healthcare API/EHR；Palantir operate |
| **G5 Launch readiness** | 观测/回滚/HITL/runbook | Healthcare launch criteria |
| **G6 Handoff** | 无原厂盯屏可运行 | Healthcare handoff |
| **G7 Productize** | ≥1 回灌候选进产品队列 | OpenAI Codify；Anthropic patterns |

**「替人把活干完」最短判据：** G1 ∧ G2 ∧ G4。

上线门补充（OpenAI Healthcare）：customer-specific benchmarks、acceptance criteria、launch readiness、handoff — 见 proposal §3.5.2。

---

## Chatbot PoC 反面（硬）

| 失败模式 | 看起来像成功 | 实际缺什么 |
|----------|--------------|------------|
| **Chatbot PoC** | 准确率演示好看 | 无工作流嵌入、无写回、无采用 → 未过 G1∧G2∧G4 |
| **Demo = Prod** | 高管现场鼓掌 | 无 Harden / eval 门 / launch criteria |
| **Utilization 人天** | 驻场很忙 | 成功变成账单小时（咨询指标冒充产品 FDE） |
| **无回灌定制** | 客户满意单点 | 不可规模复制；改牌外包 |
| **跳过 Discovery** | 很快出 Demo | 做错问题 |

红线全文 R0–R12 见 proposal **§3.7**；一页纸压缩见 [`04-one-pager.md`](04-one-pager.md)。

---

## 话术对照（硬纪律 / 软数字）

| 受众 | 宜说 | 忌说 |
|------|------|------|
| 业务赞助人 | 「这条工作流基线 X → 目标 Y；按周看采用与 KPI」 | 「我们模型很强」 |
| IT/安全 | 「权限、审计、写回、回滚、数据驻留已进 launch criteria」 | 「先接 API 再看安全」 |
| 内部产品 | 「本周回灌：模板 T / eval 失败 F / 平台缺口 P」 | 「客户特殊，无法产品化」 |
| 中国政企 | 「生产见效 + 验收条款 + 能力沉淀」 | 只甩英文「FDE」不解释采用与回灌 |

| 数字类 | 标注 |
|--------|------|
| Scale JD「节省数百小时/周」样例 | **软**（雇主样例，非独立审计） |
| 媒体 ROI% / 人工替代率 | **软 / C**；勿当合同 KPI |

---

## 飞轮一句话（硬）

| 健康 | 不健康 |
|------|--------|
| 现场模式 → playbook → 产品特性 → 下一客户更少定制 | 现场定制 → 工单 → 人月膨胀 → 产品无学习 |

---

*可转发总览：[`04-one-pager.md`](04-one-pager.md)*
