# CodeG 开源对标附录：个人 Agent 工作台 OSS 证据

| 字段 | 内容 |
|------|------|
| **日期** | 2026-10-15（Asia/Manila） |
| **性质** | 正式方案可评审附录（OSS 对标修订） |
| **主文** | [`proposal.md`](proposal.md) — **Option A 不变** |
| **调研输入** | [`/workspace/research/codeg-personal-workbench-oss-2026-10-15/`](../../research/codeg-personal-workbench-oss-2026-10-15/) |
| **星数快照** | 2026-10-15（GitHub API）；引用须带日期 |

---

## 1. 用途与约束

**用途：** 让评审者能指着开源表说清「别人怎么做、我们 Copy / Adapt / Avoid 什么」；工程可按 Option A 拆里程碑而无需再调研下列五仓。

**硬约束（本附录与主文修订均遵守）：**

| # | 约束 |
|---|------|
| 1 | **不**推倒 Option A（协作层叠现有云端 + 本机双运行时） |
| 2 | **不**重开「非开发者办公 / 开 Git 才露多代理」叙事 |
| 3 | **不**因 DSH ★ 高改为 Option B 首期换栈 |
| 4 | **不**把 OpenAgents「接尽 CLI」写成数字员工模型 |
| 5 | **不**声称 OSS 已有与 mockup 同等的「确认采用」一等公民——公开材料显示**验收普遍弱于权限**，此为 CodeG 差异化 |
| 6 | 引用 Kun 时注明 **PolyForm Noncommercial**；OpenWork 核心 **MIT**、`ee/` 另议 |
| 7 | 星数带日期 2026-10-15；不编造；不知写「未知」 |
| 8 | **不**改 `/workspace/research/...` 调研原文；**不**改 mockups / review / 原稿 PRD |

**读者用法：** Mack 已拍板 O1–O4（见 §9）；用本附录确认「抄/改/避」。产品对照八维写验收故事；工程按 §7.2 任务包挂到主文 M0–M6，**无需**再开 OSS 调研里程碑。

---

## 2. 行业方向 L0→L3 与 CodeG 落点

开源与商业产品正从三层叠进：

| 代际 | 形态 | 代表 | 人的角色 |
|------|------|------|----------|
| **L0 聊天壳** | 对话为主、工具旁路 | LibreChat / Cherry / LobeChat 类 | 一问一答 |
| **L1 Agent 工作台** | 会话 + 工具 + 权限审批 + 交付预览 | Goose、Cline、Continue、早期 coding workbench | 边看边批 |
| **L2 交办 / Cowork 工作台** | 「描述结果 → 离开 → 回来审」；进度/steer；本地文件 + MCP | Claude Cowork 叙事；Eigent；OpenWork | 交办者 + 验收者 |
| **L3 多智能体控制面** | 项目/Space 容器；主调子；Workforce / Agent Teams；多后端节点 | Eigent Workforce；cc-haha Teams；OpenAgents Workspace；OpenHands Agent Canvas；Kun ADE | 编队调度 + HITL |

**CodeG 落点：L2→L3。** Mockup 已是项目容器 + 主/子助手 + 待我处理三态 + 成果证据 + 云端\|此电脑。不是再做「更强聊天」或「隐藏多代理的办公壳」（旧 UX 简报路径已被评审与正式方案否决）。

> **一句话：** 个人 FDE 多智能体工作台——以 **Project** 为容器，以 **Digital Employee** 为可装配角色，以 **Substrate** 做委托/隔离/HITL/触发；云端与本机对等执行（Option A）；人用「待我处理」与「确认采用」闭环。

| 能力 | OSS 常见做法 | CodeG 应强化 |
|------|--------------|--------------|
| 多 Agent | Workforce / Teams / ADE / Workspace @mention | **数字员工包边界**（场景+Skills+Tools+Memory）+ 项目编队 |
| 跨端 | 多数本机桌面；OpenHands 多后端；OpenAgents 持久 URL | **云端\|此电脑** 标签 + Git 材料 / API 协作 |
| HITL | Ask first / 权限档 / HumanToolkit | **提问 / 权限 / 验收三分**，绑定 **execution attempt** |
| 证据 | Diff、usage、Kun 证据卡、OpenWork artifacts | **工件版本 + 验证依据 + 确认采用 ≠ merge** |
| UI | 固定桌面壳为主；极少开放 generative UI | **G0 固定壳对齐 mockup → G1 catalog；G2 慎用** |

### 2.1 产品形状分层（横切）

| 层 | 做什么 | 高星例子 | CodeG 启示 |
|----|--------|----------|------------|
| **Runtime / Harness** | agent loop、工具、会话日志、审批策略 | DSH、Goose、OpenHands SDK | 底座思想源；**不**首期换栈 |
| **Coding agent（IDE/CLI）** | 仓库内读写、diff、plan | Cline、Continue、Goose CLI | 深度能力保留；不是个人工作台门面 |
| **Desktop Cowork / Workbench** | 桌面壳、本地文件、MCP/Skills、交办 | Eigent、OpenWork、cc-haha、Kun | **最贴近 mockup** |
| **Collaboration OS / Control plane** | 多异构 agent 汇合、多后端切换 | OpenAgents、OpenHands Canvas、OpenWork Den | 学「节点/网关」；首期勿做成多人企业 OS |

### 2.2 数字员工定义载体（行业模式）

| 模式 | 定义载体 | 隔离 | 交接语义 | CodeG 取舍 |
|------|----------|------|----------|------------|
| 预设 Worker + MCP | Eigent Developer/Browser/Document… | 共享 Task channel；toolkit 级 | Coordinator 路由 | Adapt：角色分工；Avoid：共享可写默认 |
| Markdown Agent 定义 | cc-haha / Kun `agents/*.md` | worktree / 权限交集 | spawn / `delegate_task` | **Copy 倾向**：可版本化事实源 |
| Skills 包（SKILL.md） | Eigent、OpenWork、通用 Skills 叙事 | 作用域到 agent/workspace | 自动或显式 load | Copy：Skills 赋角色 |
| 外接异构运行时进 Workspace | OpenAgents Launcher | 每 agent 自 cwd/session | 共享线程/文件/浏览器 | Adapt：协作层；**Avoid：当员工本体** |
| 多后端 Agent Server | OpenHands Canvas ↔ Servers | Docker/VM/本机 | 切换后端不搬会话进程 | Adapt：云\|本机标签与防双跑 |

**建议吸收：** Markdown/manifest 可版本化 + Skills + MCP 工具白名单 + 记忆策略 + HITL 白名单。  
**明确不做：** 把「接到 Workspace 的任意 CLI」当成数字员工包（那是 OpenAgents 协作网）。

### 2.3 HITL 与证据：行业收敛点

- **权限侧已收敛：** Ask first / Allow / Block（OpenWork）；多档 permissionMode（cc-haha）；HumanToolkit（Eigent）；Approvals API + `request_user_input`（Kun）。  
- **验收侧普遍弱：** 公开材料未见与 mockup「确认采用」同等的一等公民业务验收；Kun 证据 verification 偏研究场景；cc-haha 偏 Diff 工程审阅。  
- **CodeG 必须保持三分领先：** 提问 ≠ 权限 ≠ 验收；确认采用 ≠ merge / 发布 / 部署（主文 §8 已写，附录再钉死）。

### 2.4 云 vs 本地（与 Option A 对齐）

| 形态 | 代表 | 启示 |
|------|------|------|
| 本机优先桌面 | Eigent、OpenWork、cc-haha、Kun、Goose | Cowork 体验基线；缺「一云一端」对等时跨现场弱 |
| 可选云 / 团队控制面 | Eigent Cloud+Enterprise；OpenWork Den（`ee/`）；OpenAgents 托管 URL | 学策略与汇合；首期勿企业化 |
| 显式多后端 | OpenHands（local / Docker / VM / Cloud） | 学切换 UX 与「不搬会话进程」 |
| **CodeG Option A** | 一云一端对等运行时 + 轻量协作 API | 比纯本机 Cowork 更接近「多后端 + 跨机汇合」的**组合**，但首期范围更小、契约更干净（Git 材料 / API 委托） |

---

## 3. 五条产品赌注

（写入主文 §0 关键立场；下文为完整表述。）

| # | 赌注 | OSS 证据锚点 | CodeG 不可妥协点 |
|---|------|--------------|------------------|
| 1 | **项目一等公民** | Eigent Space→Session→Task；OpenWork Workspace；Kun Code/Work；OpenAgents 持久 URL | Project = 约束 + 资料 + 编队 + 默认节点；侧栏 folder ≠ 项目 |
| 2 | **默认 Delegate** | Eigent Coordinator+Planner；cc-haha 主 Agent spawn；Kun `delegate_task`；OpenAgents @mention | 主助手保留合成权；Handoff 仅明确分流 |
| 3 | **HITL 分轨且绑本次运行** | OpenWork Ask/Allow/Block；cc-haha 五档；Eigent HumanToolkit；Kun approvals | 提问/权限/验收不可合并；一律绑 `execution_attempt_id` |
| 4 | **attempt 级隔离 + 跨端版本引用** | cc-haha worktree；Kun ADE/plan worktree；OpenHands per-conversation Docker | 禁共享可写活动目录；跨端靠版本引用，不搬进程 |
| 5 | **固定壳先于开放生成 UI** | 主仓均为固定壳 + MCP/Skills；DSH 为 plugin 运行时 | G0→G1；G2 慎用；HITL/验收永不走开放 HTML |

**若不坚持，会退化成什么（反例）：**

| 放弃的赌注 | 典型退化 | 为何与 OSS/mockup 冲突 |
|------------|----------|------------------------|
| 项目一等公民 | 会话列表 + 工作目录 | 无法挂编队/约束/默认节点；Eigent/OpenWork 均有容器 |
| 默认 Delegate | 到处 Handoff / 多主对话 | 合成权丢失；与 Teams「主方合成」相反 |
| HITL 分轨 | 「待你确认」大杂烩 | 权限通过被当成验收；公开 OSS 已证明权限≠验收 |
| attempt 隔离 | 共享可写活动目录 | 串代、难回滚；与 worktree/Docker 主流相反 |
| 固定壳优先 | Agent 画任意 HTML 过权限/验收 | 审计与越权风险；主仓无此先例 |

---

## 4. 相关度排名 + 星数快照

**写作/评审顺序：** cc-haha → Kun → Eigent → OpenWork → OpenAgents；对照 OpenHands / Goose / Cline / Continue / DSH。

排名按「与 CodeG mockup / Option A 同构度」而非纯星数。故 DSH（★≈249.8k）与 OpenHands（★≈90.9k）作**对照/模式**，不因星高改主推或门面。星数来自 GitHub API，快照日 **2026-10-15（Asia/Manila）**；会变，引用须带日期。

| 秩 | 仓库 | ★约（2026-10-15） | 许可 | 为何相关 | 主要借鉴 | 主要避开 |
|----|------|-------------------|------|----------|----------|----------|
| **1** | **NanmiCoder/cc-haha** | 15.0k | MIT | 同构桌面工作台：Teams / worktree / 权限 / IM | Teams 可视化；五档权限；worktree；断线保活；`agents/*.md` | bypass 默认；宠物/营销噪音 |
| **2** | **KunAgent/Kun** | 6.3k | **PolyForm Noncommercial 1.0.0** | 单运行时协议、审批、证据、delegate 降权 | `kun serve` 边界；approvals/user-inputs；证据三分离；权限只降不升 | 许可证限制商业；勿抄全实验室上下文 |
| **3** | **eigent-ai/eigent** | 15.5k | Apache-2.0 | Cowork + Workforce 概念最清晰 | Space/Session/Task；Coordinator；HumanToolkit；Skills→worker | 勿整栈换 CAMEL；共享可写需隔离规则 |
| **4** | **different-ai/openwork** | 24.1k | **MIT（核心）+ EE（`ee/`）** | 权限策略与能力 MCP Gateway | Ask/Allow/Block；能力目录 MCP | 首期 Den；EE 路径做核心协作 |
| **5** | **openagents-org/openagents** | 4.2k | Apache-2.0 | 跨机汇合；含 DSH Preview | 统一汇合面；跨机可见 | 「接尽 CLI」≠ 员工模型；headless 默默 auto |
| 对照 | OpenHands/OpenHands | 90.9k | MIT | 多后端控制中心 | 后端切换 UX；自动化触发 | 团队工程 Canvas ≠ 个人 FDE 员工包 |
| 对照 | aaif-goose/goose | 55.3k | Apache-2.0 | MCP-first 本机 agent | MCP/ACP 互通 | 非项目×员工工作台 |
| 对照 | cline/cline | 70.3k | Apache-2.0 | IDE coding agent | Plan/权限流畅度 | 停留 IDE 插件层 |
| 对照 | continuedev/continue | 36.2k | Apache-2.0 | IDE coding agent | 配置化 agent | 非 workforce |
| 模式 | deepseek-ai/deepseek-harness | 249.8k | MIT | Everything-is-plugin | plugin/session log/control room 思想 | **不**首期换 Cordis（Option B 否决） |

---

## 5. Must-cover 八维对照

| # | 主题 | OSS 证据（锚点） | CodeG 立场 | Copy | Adapt | Avoid |
|---|------|------------------|------------|------|-------|-------|
| **1** | **项目容器** | Eigent Space→Session→Task；OpenWork Workspace；Kun Code/Work 分表面；OpenAgents 持久 workspace URL | **Project 一等公民**：约束、资料、编队、默认节点 | Space/Workspace 层级语义 | 编队 + 默认执行位置进 Project Context | 退回侧栏 folder；用「工作目录」冒充项目共享上下文 |
| **2** | **数字员工** | Eigent Workers+Skills+MCP；cc-haha/Kun `agents/*.md`；OpenWork Skills/Plugins | **场景+Skills+Tools+Memory+HITL 白名单**；可按项目加载 | Markdown/manifest 可版本化；Skills 赋给角色 | 员工包 = 可安装实例 + 项目编队角色 | 把「接到 Workspace 的任意 CLI」当员工本体（OpenAgents 协作网 ≠ 员工包） |
| **3** | **编排** | Eigent Coordinator；cc-haha Teams（Lead + SendMessage）；Kun `delegate_task`；OpenAgents @mention | **默认 Delegate**；Handoff 仅明确分流 | Coordinator 路由；主方合成；delegate 权限交集 | 子任务独立卡片 + 协作树对齐 mockup 02 | 默认 Handoff；无验收标准的裸交棒 |
| **4** | **HITL** | OpenWork Ask/Allow/Block；cc-haha 五档 permissionMode；Eigent HumanToolkit；Kun `/v1/approvals` + user-inputs | **提问/权限/验收三分**；一律绑 attempt | 权限档位与审批 UI；主动问人（HumanToolkit） | 「待我处理」映射权限快照 + **验收另轨** | 合并三态；声称 OSS 已有同等「确认采用」 |
| **5** | **隔离** | cc-haha `isolation: worktree`；Kun ADE/plan worktree；OpenHands per-conversation Docker | **attempt 级**；禁共享可写活动目录 | worktree / 独立会话目录 | 能力令牌 = manifest ∩ 节点 ∩ 用户授权；无 Git 走交付目录+hash | 共享可写活动目录；伪造非 Git「隔离后采纳」 |
| **6** | **双运行时** | OpenHands 多后端；OpenAgents 跨机 workspace；OpenWork 桌面执行+云策略 | **云端\|此电脑**标签；Git=材料，API=委托/状态/介入 | 多后端切换 UX；执行分散、控制面统一 | 一云一端契约（比纯本机 Cowork 更清晰） | 首期多节点全互联；headless 关掉审批 |
| **7** | **成果证据** | Kun 证据三分离（不可变引用 / 可编辑解释 / 人工 verification）；cc-haha Diff | Preview+版本+验证；**确认采用 ≠ merge** | 证据卡语义；Diff 审阅 | 通用业务「确认采用」写进 mockup 04 不可砍 | 用 Diff/merge 冒充业务验收；砍掉验收轨去「对齐 OSS」 |
| **8** | **UI 壳 vs 生成** | 主仓均为固定壳；DSH Slots/社区 control room | **G0→G1**；G2 慎用 | 固定壳 + MCP/Skills 扩展；control room「needs you」聚合 | G1 catalog；AG-UI 事件适配 | 开放 HTML 作权限/验收主路径；首期 Cordis 换栈 |

**差异化一句：** 高星 OSS **强在权限与 Diff，弱在业务验收**；CodeG mockup 的「确认采用」应保持领先，勿学弱项。

### 5.1 八维评审要点（写进纪要可用）

1. **项目容器：** 若实现退回「侧栏文件夹 + 工作目录」，即违背赌注 1；对照 Eigent Space 与 OpenWork Workspace 的一等公民语义。  
2. **数字员工：** 首期事实源为可 Git 的 manifest（对齐 cc-haha/Kun）；Eigent 式 UI 配置可作为编辑器，但须回写同一事实源（**O1 已拍板**，见 §9）。  
3. **编排：** 主助手 Delegate 为默认；Teams/Coordinator 可视化对齐 mockup 02 协作树；Handoff 仅分流场景启用。  
4. **HITL：** 统一入口「待我处理」可以，底层类型禁止合并；每条请求展示并持久化 `execution_attempt_id`。  
5. **隔离：** attempt 级 worktree/目录 + 独立会话日志；能力令牌三维交集；无 Git 报告类走交付目录 + hash（主文已写）。  
6. **双运行时：** UI 标签「云端 | 此电脑」；材料走 Git 版本引用；委托/状态/介入走 Collaboration API；首期一云一端，防双跑。  
7. **成果证据：** Preview / Changes / Source + 版本 hash + 验证依据；子任务完成只回填卡片，主项须「确认采用」。  
8. **UI：** G0 对齐四张 mockup；G1 catalog 受控扩展；G2 仅只读试点；HITL/验收永不走开放 HTML。

### 5.2 与 mockup 要素对齐速查

| Mockup 要素 | 最贴近的 OSS 证据 | 相对 OSS 的 CodeG 差距 |
|-------------|-------------------|------------------------|
| 项目容器 | Eigent Space；OpenWork Workspace；Kun Code/Work | 坚持「约束+编队」一等公民，勿退回 folder |
| 主/子助手树 | Eigent Canvas；cc-haha Teams；Kun ADE | 已有 IA；补员工包与 attempt 绑定 |
| 云端 \| 此电脑 | OpenHands 后端切换；OpenAgents 跨机 | **契约更清晰**（Git+API）；需落地防双跑 |
| 待我处理三态 | 权限多方强；**验收普遍弱** | **保持三分领先** |
| 成果与证据 | Kun 证据卡；cc-haha Diff | 通用「确认采用」仍稀缺——保留 |
| 跨端版本交接 | OpenAgents 共享文件；OpenHands 不搬进程 | 对齐正式方案：版本引用 |
| 补充指令送达 | Kun steering；cc-haha 队列/断线语义 | 主文已写立即/下一轮/队列——对照实现 |

---

## 6. 分仓卡片

### 6.1 五主仓（精炼）

#### cc-haha（NanmiCoder/cc-haha）— ★15.0k · MIT · 相关度最高

与 CodeG **同构度最高**的开源桌面 coding workbench：Teams、worktree、权限档、IM 接力、Computer Use 均在公开材料中可见。

| 维度 | 事实 |
|------|------|
| 形状 | Electron 桌面 + Bun sidecar server + CLI 子进程 + IM adapters；H5 远程 |
| 项目 | 多会话 / 项目切换；分支或 **Git Worktree** 启动会话 |
| 多 Agent | 内置多类 SubAgent；自定义 `~/.claude/agents/*.md`；**Agent Teams**（Lead + SendMessage + shutdown）；动态 Workflow |
| 工具 | MCP（STDIO/HTTP/SSE，项目/共享/全局作用域）；Skills 市场；Computer Use；应用内浏览器预览 |
| HITL | 五档 permissionMode（含 default / plan / acceptEdits / bypass / dontAsk 等）；图形化审批；危险命令与 AI 反问；IM 远程审批 |
| 隔离 / 证据 | `isolation: worktree`；文件 Diff、整轮撤销、模型请求追踪、会话活动面板 |
| 云/本地 | 本地优先；H5/IM 接力；客户端断线 ≠ 停任务（宽限期） |
| UI | 固定工作台；Teams 泳道可视化；非 generative UI |
| Copy | Teams 通信与关停协议；worktree；权限档；断线保活；Markdown agent 事实源 |
| Adapt | 「待我处理」← 权限请求快照 + **验收另轨**；跨端标签可借鉴 H5/IM，但执行仍绑节点 |
| Avoid | bypass 当默认；桌面宠物当交付；IM 权限面过宽；功能堆砌而无 Project/Employee 对象模型 |

#### Kun（KunAgent/Kun）— ★6.3k · **PolyForm Noncommercial 1.0.0**

> **许可警告：** PolyForm Noncommercial——商业产品仅作**架构参考**，不可直接搬代码进商业发行。

单运行时 GUI+TUI、审批协议、证据工作区、delegate 权限只降不升——对 Substrate 边界设计参考价值**很高**。

| 维度 | 事实 |
|------|------|
| 形状 | 桌面 GUI + TUI，**同一 `kun serve` 运行时**；明确反对「运行时切换器」产品表面 |
| 项目 | Code（含 Design 任务）/ Work 表面；`additionalWorkspaces`；知识库只读挂载 |
| 多 Agent | 产品表面偏单一 Kun agent + **subagent profiles** / `delegate_task`；ADE（总管+worker）；可嵌外部 harness |
| 工具 | MCP / Skills；sandbox；Computer Use（仅 GUI surface） |
| HITL | `approvalPolicy`；`/v1/approvals`、`/v1/user-inputs`；子代理权限与父级**只降不升** |
| 记忆/证据 | 本地记忆（不可信 reference）；**论文证据工作区**：不可变引用 / 可编辑解释 / 人工 verification 三分离 |
| 云/本地 | 本地优先；手机远程；定时；关闭主窗口即退出整套服务 |
| Copy | 单一协议边界；clientSurface；delegate 权限交集；证据三分离；GUI ≠ 第二个 agent loop |
| Adapt | CodeG Substrate 对标审批/事件面与 attempt 绑定，**不**搬 Electron / 不换许可证栈 |
| Avoid | 商业发行依赖 Kun 代码；复制全部上下文窗口实验；Rooms/订阅 SDK 复杂例外当首期 |

#### Eigent（eigent-ai/eigent）— ★15.5k · Apache-2.0

开源 **Cowork + Workforce** 叙事最清晰：Space 层级、Coordinator/Planner、预置 Worker、HumanToolkit、Skills→worker 均可直接对照 mockup 项目编队。

| 维度 | 事实 |
|------|------|
| 形状 | Electron 桌面（React）；Local / Cloud-connected / Enterprise / Cloud |
| 项目 | **Space → Session → Task**；Workforce 在 Session 内处理 Tasks |
| 多 Agent | CAMEL Workforce：Coordinator + Task planner + Worker nodes；预置 Developer / Browser / Document / Multi-Modal；Custom worker + MCP |
| 工具 | MCP Connectors；Skills（SKILL.md）；内置 Browser/Terminal/Document 等 Toolkit |
| HITL | **HumanToolkit**（卡住/凭据/主观决策时问人）；浏览器可 Take Control |
| 记忆/证据 | NoteTakingToolkit 跨 agent 共享 Markdown 笔记；Canvas 展示子任务状态与日志 |
| 云/本地 | 推荐本地部署；亦有云与企业 SSO |
| UI | 固定桌面壳 + React Flow；非 generative UI |
| Copy | Space 层级；Workforce 角色分工；HumanToolkit；Skills 可赋给特定 worker |
| Adapt | 「数字员工」= 可版本化 worker 定义 + toolkit 白名单；Canvas → CodeG 协作树 |
| Avoid | 把 CAMEL 整栈当底座（违背 Option A）；共享笔记目录若可写冲突则缺隔离规则 |

#### OpenWork（different-ai/openwork）— ★24.1k · MIT（核心）+ EE（`ee/` Den）

> **许可：** 核心 MIT；团队控制面在 `ee/`（Den）——**另议**，首期勿把 EE 路径做成协作核心。

星最高的开源 Cowork 桌面之一；权限策略与「能力经 MCP Gateway 复用」成熟。公开文档对 Workforce 树弱于 Eigent（矩阵标「偏单工作区 agent」）。

| 维度 | 事实 |
|------|------|
| 形状 | Electron 桌面（macOS/Windows/Linux，基于 OpenCode）；非桌面亦可经 **MCP Gateway** 被其他 agent 调用 |
| 项目 | Workspace；本地 skills 在 `.opencode/skills/`；workspace MCP |
| 多 Agent | 偏「单工作区 agent + 可共享能力」；组织侧发布 skills/plugins；公开 README **未强调**多 worker 树 |
| 工具 | MCP；Skills；Google/Microsoft 等连接；OpenWork MCP：`search_capabilities` / `execute_capability` / `list_skills` / `get_skill` |
| HITL | **Agent permissions**：命令/改文件/网站/搜索/本地 skills/本地 MCP → Allow / Ask first / Block；团队取最严 |
| 记忆/证据 | Skills/connectors 组织分发；MCP Apps / artifacts（文档描述） |
| 云/本地 | 本地免费可用；Den 为团队控制面（推理、策略、市场、RBAC） |
| Copy | 权限三维矩阵；「能力目录 MCP」让外部 agent 复用同一套 skills |
| Adapt | CodeG 云节点可对本地节点暴露「只读能力目录」；策略离线缓存思路；组织策略是否进首期见 O2 |
| Avoid | 首期做完整 Den；依赖 EE 许可路径做核心协作；企业治理压过个人 FDE |

#### OpenAgents（openagents-org/openagents）— ★4.2k · Apache-2.0

「协作 OS」叙事：异构 agent 汇合到持久 Workspace；含 **DeepSeek Harness Preview**（注明 Preview，不据此换栈）。对 CodeG「云端\|此电脑」跨机可见有启发，但**不是**数字员工模型。

| 维度 | 事实 |
|------|------|
| 形状 | **Workspace（Web）** + **Launcher（桌面/CLI `agn`）** + Network SDK |
| 项目 | 持久 workspace URL；共享 threads / files / browser / tunnels |
| 多 Agent | 异构 coding agents 接入同一 workspace；@mention 委托；Goose 等标 Beta；**DSH 为 Preview** |
| 工具 | 共享浏览器与文件；各 agent 自带工具；MCP/A2A（SDK 层） |
| HITL | Workspace Stop；部分 adapter（如 Goose）headless 强制 `GOOSE_MODE=auto`——**人机审批在 headless 路径被削弱**（文档明示） |
| 记忆/证据 | 共享上传文件；每 agent 独立 session 映射 |
| 云/本地 | 可本地 agent + 托管/自建 workspace |
| Copy | 「执行位置分散、控制面统一」；跨机汇合叙事对齐 CodeG 云\|本机 |
| Adapt | 把 Workspace 想成协作层而非运行时；员工仍在各自 runtime；「只读聚合视图」是否产品化见 O3 |
| Avoid | 用「接所有 CLI」替代数字员工包；在 headless 路径默默关掉审批；首期做成必须接 10+ CLI 的中心 |

### 6.2 对照仓短表

| 仓库 | ★约 | 形状 | 与工作台关系 | CodeG 用法 |
|------|-----|------|--------------|------------|
| OpenHands | 90.9k | Agent Canvas Web 控制中心；多 Agent Server；Automations | **多节点切换**模板 | 学后端切换 UX 与自动化触发；员工包模型弱，勿做成团队工程 Canvas 替代个人 FDE |
| Goose | 55.3k | 桌面+CLI+API；MCP 为核心扩展；ACP | 强 runtime，弱项目容器 | 学 MCP/ACP 互通；不是项目×员工工作台门面 |
| Cline | 70.3k | IDE 扩展 / SDK / CLI | Coding agent 流畅度 | 学 Plan/权限流；停留 IDE 插件层 |
| Continue | 36.2k | IDE coding agent | 同上 | 学配置化 agent；非 workforce |
| DSH | 249.8k | Cordis 插件树；profiles（web/headless/sdk/acp） | **架构模式**；社区 Workbench/Worktable ≠ 官方唯一 UI | 学 plugin/session log/control room；**明确不首期换 Cordis** |

**可选短注：** LibreChat / Cherry / LobeChat = L0 聊天壳，不可当 L2/L3 对标主仓。AutoGen Studio / CrewAI UI = 编排工作室，缺个人 FDE 桌面、待我处理三态与云端\|本机协作契约。

**对照仓共同结论：** 高星不等于个人工作台形态。OpenHands/Goose/Cline/Continue/DSH 分别贡献「多后端」「MCP」「IDE 流畅度」「配置化」「plugin 思想」；**门面与对象模型仍以五主仓 + mockup 为准**，并落在 Option A 上。

---

## 7. Copy / Adapt / Avoid 一页

| | Copy | Adapt | Avoid |
|--|------|-------|-------|
| **cc-haha** | Teams；worktree；权限档；断线保活；agents.md | IM/H5 作通知面非第二真相 | bypass 默认；功能堆砌无对象模型 |
| **Kun** | serve 边界；审批 API；delegate 降权；证据三分离 | Substrate 对标协议 | 换许可证栈；抄全实验室上下文 |
| **Eigent** | Space 层级；HumanToolkit；Skills→worker | Worker→数字员工 manifest | 整栈 CAMEL |
| **OpenWork** | Ask/Allow/Block；能力 MCP Gateway | 策略下发到双节点 | 首期 Den/EE 核心化 |
| **OpenAgents** | 统一汇合面；跨机可见 | 协作层≠员工定义 | headless 默默 auto；接尽 CLI |
| **OpenHands** | 多后端 UX | 云\|本机切换文案 | 团队工程 Canvas 替代个人 FDE |
| **DSH** | plugin/profile；session log；control room 概念 | Option A 能力缝 | Option B 首期换 Cordis |

### 7.1 取舍理由（评审口述稿）

- **为何 Copy cc-haha Teams / worktree：** 与 mockup 02 协作树、attempt 隔离同构；实现路径短，不依赖换栈。  
- **为何 Copy Kun 证据三分离与审批 API：** 补齐「引用不可变 / 解释可编辑 / 人验证独立」语义；审批绑运行时边界，对齐 Substrate HITL 门——但**代码不可商业搬迁**（PolyForm NC）。  
- **为何 Adapt Eigent Worker 而非换 CAMEL：** 要的是「可装配角色 + Skills/MCP 白名单」产品对象，不是编排框架迁移。  
- **为何 Adapt OpenWork 权限矩阵而非首期 Den：** 个人节点本地档位即可验收；组织最严策略可后置（O2）。  
- **为何 Avoid OpenAgents「接尽 CLI」：** 那是协作网关，混淆后数字员工包边界消失，与主文 §5 冲突。  
- **为何 Avoid DSH 首期换栈：** ★ 高证明 plugin/session 思想有共鸣，不证明 Cordis 置换个人 FDE 云本地 Git 协作更便宜（主文 §3.2 / §12）。

### 7.2 可拆工程任务（对齐主文里程碑，不另立换栈线）

| 任务包 | 吸收的 OSS 模式 | 落点里程碑（主文 §14） |
|--------|-----------------|------------------------|
| Employee manifest schema（含 skills/tools/hitl 白名单） | cc-haha/Kun agents.md；Eigent worker 字段 | M0 |
| Project Context（约束/资料/编队/默认节点） | Eigent Space；OpenWork Workspace | M0–M2 |
| Delegate API + 协作树 UI | Eigent Coordinator；cc-haha Teams；Kun delegate | M1–M2 |
| HITL 三态 + attempt 绑定 | OpenWork 权限；Kun approvals；mockup 03 | M3 |
| 成果 Preview + 确认采用（≠ merge） | Kun 证据语义；cc-haha Diff；mockup 04 | M3 |
| attempt worktree/目录 + 能力令牌 | cc-haha worktree；Kun ADE；OpenHands Docker 思想 | M5 |
| 云\|本机标签 + sync_wait | OpenHands 多后端；OpenAgents 跨机；主文 Git+API | M1–M2 |
| G0 壳对齐 mockup；G1 catalog 预留 | 主仓固定壳；DSH control room 思想 | M2、M6 |

---

## 8. 对 Option A 底座的含义（强化而非推翻）

正式方案 Option A（协作层叠双运行时）与 OSS 证据**一致**，本附录强化论证、**不**改主推：

1. **不要换 runtime 宗教。** 高星工作台各自绑 CAMEL / OpenCode / Claude Code fork / kun serve / Cordis；胜出因素是 **IA + HITL + 隔离 + 可装配能力**，不是统一 harness。→ 继续 Option A；DSH 仅思想借鉴（主文 §12）。  
2. **协作层对象可直接映射：**

| CodeG 对象 | OSS 近似 |
|------------|----------|
| Project | Eigent Space / OpenWork Workspace / Kun Code·Work / OpenAgents 持久 URL |
| Digital Employee | Eigent Worker / cc-haha·Kun `agents/*.md` / profile |
| Execution attempt | 会话 / 子任务 run / worktree 会话 |
| Node | OpenHands Agent Server / OpenAgents Launcher 实例 / 云\|本机 |
| HITL gate | OpenWork Ask first / cc-haha 权限档 / Kun approvals / Eigent HumanToolkit |

3. **双运行时有先例组合：** OpenHands 多后端 + OpenAgents 跨机 workspace + OpenWork「桌面执行 + 云策略」。CodeG 的 Git（材料）+ API（委托/状态/介入）是更干净的**个人规模**契约；首期范围小于企业 Den / 全互联。  
4. **验收语义应继续领先：** OSS 强权限与 Diff，弱「业务确认采用」；mockup 04 与主文 §8「确认采用 ≠ merge」**不可砍**。勿为「对齐 OSS」删掉验收轨。  
5. **UI：** 全部主仓证明 **固定壳 + MCP/Skills/插件** 足够扩展；DSH Slots/社区 control room 作 G1 灵感，不作开放 HTML；**不**因 DSH ★≈250k 首期换 Cordis。

**工程含义：** 里程碑仍按主文 §14（M0–M6）；本附录不引入「先迁 Cordis」或「先做企业 Den」分叉。对象模型与协作 API 可按上表从 OSS 映射验收，而无需再调研五仓源码。

### 8.1 四张 mockup × OSS 证据（验收口述）

| Mockup | 人在做什么 | OSS 最近似能力 | CodeG 必须多出来的 |
|--------|------------|----------------|-------------------|
| 01 项目工作台 | 选项目、看编队与节点、扫待处理与成果 | Eigent Space 列表；OpenWork Workspace；cc-haha 项目切换 | Project 约束/资料/编队一等公民；云端\|此电脑在线标签 |
| 02 多 Agent 协作 | 看委托树、补指令、看跨端版本 | Eigent Canvas；cc-haha Teams；Kun ADE | Delegate 默认 + attempt 绑定；版本引用时间线（不搬进程） |
| 03 待我处理 | 分轨处理提问/权限/验收 | OpenWork Ask；cc-haha 五档；Kun approvals（验收侧普遍弱） | **验收独立 Tab**；全部绑本次运行 |
| 04 成果与证据 | Preview、提修改、确认采用 | Kun 证据卡；cc-haha Diff | **确认采用 = 业务验收 ≠ merge**；验证依据进证据 |

### 8.2 否决清单（评审可勾）

| 提议 | 本附录立场 |
|------|------------|
| 因 DSH 星高改 Option B 首期 | **否决** |
| 用 OpenAgents「接尽 CLI」定义数字员工 | **否决** |
| 声称 OSS 已有同等「确认采用」 | **否决**（公开材料不支持） |
| 首期企业 Den / 组织治理主轴 | **否决**（个人 FDE 优先；见 O2） |
| 推倒 Option A 换统一 harness | **否决** |
| 重开「非开发者办公 / 开 Git 才露多代理」 | **否决**（主文 §1.2 / §1.4 已否决） |

---

## 9. 开放问题 → 已决议（2026-10-15）

O1–O4 原为开放问题；**Mack 已全部按建议拍板**（2026-10-15）。主文 **§15**「已决议」与下表一致；**Option A 不变**。原 §14/#1–10 问题仍见主文（未在本附录重开）。

### 9.1 来自本轮 handoff（OSS 相关）— **已拍板**

| # | 问题 | 决议内容 | 状态 | 曾标风险（已规避） |
|---|------|----------|------|-------------------|
| O1 | 数字员工事实源：偏 cc-haha/Kun 的 Markdown manifest，还是 Eigent 式 UI 配置 + 导出？ | **Markdown/manifest 为数字员工事实源**；UI 为编辑器，导出回写 manifest | **已拍板** | UI-only 配置难审难迁；与项目 Git 材料脱节 |
| O2 | 组织级权限策略（OpenWork 型）是否进入个人产品首期，还是仅节点本地档位？ | 首期 **仅节点本地权限档位**；组织级策略后置 | **已拍板** | 首期做 Den 会拖垮一云一端主路径 |
| O3 | OpenAgents 式「持久汇合 URL」与 CodeG「一云一端」是否要显式产品化第三种「只读聚合视图」？ | 首期 **不做** OpenAgents 式「只读聚合视图」；后期可选 | **已拍板** | 多一个产品表面，分散防双跑与 HITL 绑定精力 |
| O4 | Computer Use（cc-haha/Kun）是否进数字员工工具白名单首期？ | Computer Use 为 **可选能力、默认关**；须节点能力声明 + 用户对本 attempt 授权 | **已拍板** | 默认可控面过宽；与「节点能力声明」原则冲突 |

**决议语义（与主文一字对齐）：**

1. **O1：** Markdown/manifest 为数字员工事实源；UI 为编辑器，导出回写 manifest  
2. **O2：** 首期仅节点本地权限档位；组织级策略后置  
3. **O3：** 首期不做 OpenAgents 式「只读聚合视图」；后期可选  
4. **O4：** Computer Use 为可选能力、默认关；须节点能力声明 + 用户对本 attempt 授权

### 9.2 调研缺口（勿过度推断）

- 未逐行审计各仓源码；卡片以官方 README/docs 为准。「○ 未见」≠ 产品一定没有，仅表示公开材料未建立该一等能力。  
- OpenWork 多 Agent「workforce 树」公开文档弱于 Eigent——矩阵标「偏单工作区 agent」。  
- Eigent docs.eigent.ai 首页曾 500；Workforce/MCP/Skills 子页可访问。  
- Kun 商业使用受 **PolyForm Noncommercial** 限制——**仅作架构参考**。  
- DSH Agent Teams / 与 ACP 字节兼容未在本轮验证；社区 Workbench/Worktable ≠ 官方唯一 UI。  
- OpenAgents 中 Goose/DSH 标注 Beta/Preview——引用时须带状态，不可写成已稳定员工运行时。  
- 星数会变；引用请带 **2026-10-15** 日期（见 [`sources.md`](../../research/codeg-personal-workbench-oss-2026-10-15/sources.md)）。

---

## 10. 参考来源

### 本轮 OSS 调研包（必读）

| 文件 | 内容 |
|------|------|
| [`handoff-to-方案撰写.md`](../../research/codeg-personal-workbench-oss-2026-10-15/handoff-to-方案撰写.md) | 交接约束与 Must-cover |
| [`research-brief.md`](../../research/codeg-personal-workbench-oss-2026-10-15/research-brief.md) | 方向 + 赌注 + 分仓卡片 |
| [`oss-matrix.md`](../../research/codeg-personal-workbench-oss-2026-10-15/oss-matrix.md) | 维度对照与 Copy/Adapt/Avoid |
| [`sources.md`](../../research/codeg-personal-workbench-oss-2026-10-15/sources.md) | URL、星数快照、方法限制 |

**星数快照摘录（2026-10-15，详 sources）：** OpenWork 24108 · Eigent 15542 · cc-haha 15014 · Kun 6315 · OpenAgents 4210 · OpenHands 90924 · Cline 70337 · Goose 55289 · Continue 36226 · deepseek-harness 249762。本附录正文用约数（如 24.1k）；精确值以 sources 表为准。

### 方案包与上游（本机）

| 路径 | 角色 |
|------|------|
| [`proposal.md`](proposal.md) | 正式方案主文（Option A） |
| [`ui-journeys.md`](ui-journeys.md) | UI 旅程 |
| [`../codeg-personal-workbench-mockups-2026-10-14/`](../codeg-personal-workbench-mockups-2026-10-14/) | IA 基线 |
| [`../../research/codeg-agent-substrate-digital-employees-2026-10-14/`](../../research/codeg-agent-substrate-digital-employees-2026-10-14/) | 前次底座/AG-UI/DSH 调研 |
| [`../codeg-personal-work-prd-review-2026-10-14/cloud-local/proposal.md`](../codeg-personal-work-prd-review-2026-10-14/cloud-local/proposal.md) | 云端协作契约 |

### 外部入口（可点击；细节见 research pack sources）

| 仓库 / 文档 | URL |
|-------------|-----|
| Eigent README | https://github.com/eigent-ai/eigent |
| Eigent Workforce | https://www.eigent.ai/docs/workforce |
| OpenWork README | https://github.com/different-ai/openwork |
| OpenWork Agent permissions | https://openworklabs.com/docs/cloud/share-with-your-team/agent-permissions |
| OpenAgents README | https://github.com/openagents-org/openagents |
| cc-haha README | https://github.com/NanmiCoder/cc-haha |
| Kun README（**PolyForm NC**） | https://github.com/KunAgent/Kun |
| Kun architecture | https://raw.githubusercontent.com/KunAgent/Kun/master/docs/kun-architecture.md |
| Kun 证据工作区 | https://raw.githubusercontent.com/KunAgent/Kun/master/docs/paper-evidence-workspace.md |
| OpenHands | https://github.com/OpenHands/OpenHands |
| Goose | https://github.com/aaif-goose/goose |
| Cline | https://github.com/cline/cline |
| Continue | https://github.com/continuedev/continue |
| DeepSeek Harness | https://github.com/deepseek-ai/deepseek-harness |

### 说明

- 本附录不编造星数或未证实能力；「○ 未见」仅表示公开材料未建立该一等能力。  
- 未改动 research 原文、mockups、review、原稿 PRD。  
- **Option A 为主路径；本附录是证据与对标，不是换栈提案。**

### 完成标准（自检）

| 检查项 | 状态 |
|--------|:----:|
| Must-cover 八维均有 OSS 锚点 + CodeG 立场 + Copy/Adapt/Avoid | ✅ |
| 五条赌注写入且与主文 §0 一致 | ✅ |
| 相关度顺序：cc-haha → Kun → Eigent → OpenWork → OpenAgents | ✅ |
| 星数带日期 2026-10-15；Kun 标 PolyForm NC；OpenWork 标 MIT+`ee/` | ✅ |
| **不**推倒 Option A；**不**因 DSH ★ 高换栈 | ✅ |
| **不**重开非开发者办公 / 开 Git 才露多代理 | ✅ |
| **不**把 OpenAgents「接尽 CLI」写成数字员工 | ✅ |
| **不**声称 OSS 已有同等「确认采用」 | ✅ |
| handoff O1–O4 进入主文并 **已拍板** | ✅（主文 §15「已决议」；本附录 §9） |

---

*附录结束。*
