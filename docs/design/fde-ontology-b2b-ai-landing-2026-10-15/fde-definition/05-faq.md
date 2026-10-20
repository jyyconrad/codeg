# 05 — 常见误解 FAQ

| 字段 | 内容 |
|------|------|
| **用途** | 对齐口径；每条给「为何错」+「正确说法」 |
| **对应** | proposal §2 / §3；research `01`/`03` |
| **标注** | 结论均为**硬**口径，除非标明软 |

---

## Q1：SA / Solutions Engineer = FDE？

| | |
|--|--|
| **错** | 解决方案架构师做了 PoC，就算 FDE |
| **对（硬）** | SE/SA 重心在售前→早期售后；**不以生产运维所有权为主**。FDE 卖生产采用 + 回灌。阿里云 JD 已写「**协同 FDE 团队**」——显式分工。**SA 默认 ≠ FDE**，除非 JD 含生产码与上线所有权 |
| **见** | [`01-what-is-fde.md`](01-what-is-fde.md)、[`02-capability-four-pillars.md`](02-capability-four-pillars.md) |

---

## Q2：Demo 成功 = 生产落地？

| | |
|--|--|
| **错** | 高管鼓掌的 Demo 可直接当 Prod |
| **对（硬）** | **Demo ≠ Prod**。须过 Harden / 金标 eval / launch criteria / handoff。Demo 必须标「非生产」+ 环境剖面。红线 R2 |
| **见** | [`03-value-get-work-done.md`](03-value-get-work-done.md)、proposal §3.7 / §7 |

---

## Q3：人天忙 = 成功？

| | |
|--|--|
| **错** | 驻场 utilization 高 = FDE 成功 |
| **对（硬）** | 成功度量 = **adoption + workflow impact + eval**（OpenAI 三角），不是账单小时。Utilization 是**咨询指标**；拿它冒充产品 FDE = 红线 R12 |
| **见** | [`03-value-get-work-done.md`](03-value-get-work-done.md) |

---

## Q4：会调模型 / 准确率高 = 把活干完？

| | |
|--|--|
| **错** | Chatbot 答题准确率好看即可验收 |
| **对（硬）** | 「替人把活干完」= **G1∧G2∧G4**（采用 ∧ 工作流指标 ∧ 写回）。缺写回仍是「会答题的 Demo」（红线 R0） |
| **见** | [`03-value-get-work-done.md`](03-value-get-work-done.md)、[`04-one-pager.md`](04-one-pager.md) |

---

## Q5：TDL / 交付负责人也要写生产码？

| | |
|--|--|
| **错** | 所有前线角色都应写码 |
| **对（硬）** | Anthropic **TDL JD 明文：won't write production code**；alongside FDEs who build。TDL 管 SOW、干系人、ROI 前后测。一人公司可兼；并行客户 ≥2 应拆看板 |
| **见** | [`01-what-is-fde.md`](01-what-is-fde.md)、proposal §3.1 |

---

## Q6：中国没有 FDE，所以模型不适用？

| | |
|--|--|
| **错** | 只有美国公司叫 FDE 才算 |
| **对（硬）** | 商汤猎聘已用职衔「**FDE 工程师**」（A）。多数用驻场/交付/建设服务完成**同构三角**（场景共建、系统集成、能力沉淀）。对内统一 FDE，对外可双语 |
| **软** | 媒体案例数字勿当合同 KPI |
| **见** | [`02-capability-four-pillars.md`](02-capability-four-pillars.md) |

---

## Q7：客户满意单点定制 = 合格 FDE engagement？

| | |
|--|--|
| **错** | 客户口头满意即可结项 |
| **对（硬）** | 缺 **G7 回灌** = 不可规模复制，接近改牌外包（红线 R1/R6）。每里程碑 ≥1 回灌候选进产品队列 |
| **见** | [`01-what-is-fde.md`](01-what-is-fde.md)、[`03-value-get-work-done.md`](03-value-get-work-done.md) |

---

## Q8：本拆读册会不会改掉 CodeG Option A？

| | |
|--|--|
| **错** | 定义加厚 = 换栈 / 推倒 Option A |
| **对（硬）** | **不推倒** CodeG Option A 五条产品赌注。本夹只拆读 FDE 定义/价值；Ontology/工作台/Agent 仍挂在 Option A 之上的运营层（proposal §0） |
| **见** | [`../proposal.md`](../proposal.md) §0；[`../README.md`](../README.md) |

---

## Q9：内部支撑 / 调参 = FDE？

| | |
|--|--|
| **错** | 内线帮忙调参、答疑、人天忙 = 已按 FDE 交货；或「内线只做人天、外线才叫 FDE」 |
| **对（硬）** | **内线也要** G1∧G2∧G4（采用 ∧ 工作流指标 ∧ 写回），且 **G7 回灌进对应品类/产品 backlog**。赞助人 = 部门产品 Owner / 业务方。禁止把「帮忙调参」当交货（见 06 红线 I3） |
| **见** | [`06-internal-business-four-capabilities.md`](06-internal-business-four-capabilities.md)、[`03-value-get-work-done.md`](03-value-get-work-done.md) |

---

*FAQ 结束。回 [`04-one-pager.md`](04-one-pager.md) 或 proposal §2/§3；内线四类能力 → [`06-internal-business-four-capabilities.md`](06-internal-business-four-capabilities.md)。*
