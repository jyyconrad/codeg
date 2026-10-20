# CodeG「通用 Agent 调度底座 × 数字员工 × 多项目编队」方案包

**日期：** 2026-10-14（Asia/Manila）  
**状态：** 正式方案草案索引（正文见 `proposal.md`）

## 读者

| 读者 | 用途 |
|------|------|
| Mack | **O1–O4 已拍板**（见 proposal §15「已决议」）；其余开放问题（§15 #1–10）仍可确认；Option A 与非目标已定 |
| 产品 | 按方案章节拆需求与验收故事 |
| 工程 | 按底座能力清单 / 里程碑拆任务 |

## 阅读顺序

1. 本 README（定位与材料关系）  
2. [`proposal.md`](proposal.md) — 正式方案正文（§0–§15；主推 Option A）  
3. [`oss-benchmark.md`](oss-benchmark.md) — **开源对标附录**（五条赌注、八维 Must-cover、Copy/Adapt/Avoid；**不改 Option A**）  
4. [`ui-journeys.md`](ui-journeys.md) — 界面草图 / 用户旅程 / 操作示意（§10 可视设计层）  
5. 需要细节时再回 mockups 原图与调研包（见下）

## 与调研包的关系

| 材料 | 关系 |
|------|------|
| [`oss-benchmark.md`](oss-benchmark.md) | **本包附录**：个人工作台 OSS 对标（2026-10-15）；强化 Option A |
| [`/workspace/research/codeg-personal-workbench-oss-2026-10-15/`](../../research/codeg-personal-workbench-oss-2026-10-15/) | **输入（OSS）**：research-brief / oss-matrix / sources / handoff；**不修改**调研原文 |
| [`/workspace/research/codeg-agent-substrate-digital-employees-2026-10-14/`](../../research/codeg-agent-substrate-digital-employees-2026-10-14/) | **输入（底座）**：调研 brief、来源、交接成功标准；本方案据此写成，**不修改**调研原文 |
| [`ui-journeys.md`](ui-journeys.md) | **界面与旅程专章**（线框、Journey、Task flows；嵌入四张概念稿） |
| [`../codeg-personal-workbench-mockups-2026-10-14/`](../codeg-personal-workbench-mockups-2026-10-14/) | **UI IA 基线**（四张 mockup 原图与 prompts） |
| [`../codeg-personal-work-prd-review-2026-10-14/review.md`](../codeg-personal-work-prd-review-2026-10-14/review.md) | **定位与验收故事**来源（FDE；否决原稿偏移） |
| [`../codeg-personal-work-prd-review-2026-10-14/cloud-local/proposal.md`](../codeg-personal-work-prd-review-2026-10-14/cloud-local/proposal.md) | **跨端架构**直接吸收（Git+API、一云一端） |
| [`../codeg-personal-work-prd-2026-10-14/`](../codeg-personal-work-prd-2026-10-14/) | **历史 PRD**，仅作对照差异；勿原样开工 |

## 一句话立场

主推 **Option A**（协作层叠现有云端+本机双运行时）；数字员工 = Manifest；项目 = Context + 编队 + Work items；对内定位为 **个人 FDE 多智能体工作台**。Option B（DSH/Cordis 换栈）仅对照，不进首期。OSS 对标见 [`oss-benchmark.md`](oss-benchmark.md)。
