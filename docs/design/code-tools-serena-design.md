# Codeg「代码工具」设计方案：LSP、CodeGraph 与 Serena

| 字段 | 内容 |
| --- | --- |
| 文档标题 | Codeg「代码工具」设计方案：LSP、CodeGraph 与 Serena |
| 日期 | 2026-09-22 |
| 状态 | 待评审；本文件只定义方案，不代表代码已实现 |
| 受众 | Codeg 产品、前端、Rust 后端与 MCP 集成开发者 |
| 相关入口 | `/settings/code-intelligence`、`src-tauri/src/agent/code_intel/`、`src/components/settings/code-intelligence-settings.tsx` |
| 当前事实 | 现有页面和后端维护 LSP 进程池、CodeGraph 子进程和项目级 MCP 适配器 |

## 1. 背景与目标

当前 Codeg 的代码智能能力由两套 Codeg 自有工具实现：Rust 后端维护 LSP 进程池，CodeGraph 由 Codeg 启动并代理，Codeg Agent 还会额外收到 `lsp` 和 `codegraph` 两个 Native Rig 工具。设置页展示的是内部实现细节，用户需要理解二进制路径、索引目录和语言服务器进程，才能判断工具是否可用。

本方案把设置入口改名为「代码工具」，让页面负责维护和启动三类外部 MCP 工具：

1. 保留「代码智能」总开关。
2. 保留 LSP、CodeGraph、Serena 三个互相独立的选项。
3. 勾选工具后，Codeg 先查找系统已有的官方命令或 MCP 配置；找不到时，按工具登记的官方安装方式自动下载到 Codeg 管理目录。
4. 每个工具的 MCP 启动参数直接依据其官方配置方式生成，Codeg 不重写工具协议或私自拼接另一套命令。
5. 所有选中的工具按工作区启动，并提供给当前工作区内的外部 ACP Agent 和 Codeg Agent。
6. LSP 继续支持按开发语言选择，并限制同时运行的 LSP 数量。
7. 配置页沿用「智能体」单个配置页的结构：页头说明、工具列表、单项详情、状态与保存操作分离。

## 2. 范围与非目标

### 2.1 本期范围

- 将导航文案从「Tools 配置」改为「代码工具」；英文及其他语言同步使用对应的自然表达。
- 把原 `CodeIntelConfig` 重构为三类 MCP 工具的配置和运行状态。
- 实现按工作区去重的 MCP 工具 supervisor。
- 实现全局发现、Codeg 管理目录发现、官方安装器下载、版本检查和启动失败提示。
- 把官方 MCP 工具动态注入当前工作区的 Agent 会话。
- 保留 LSP 的语言选择、语言检测和最大并发数。
- 为已有 `code-intel.json` 提供一次性兼容迁移。
- 在文档和页面中明确网络、下载、权限和启动失败状态。

### 2.2 非目标

- 不在 Codeg 内重新实现 LSP 协议、语言分析器、代码图谱或 Serena 的语义工具。
- 不执行 `codegraph install`、`serena setup <client>` 等会修改其他 Agent 家目录配置的命令。
- 不把工具安装到系统目录，不覆盖用户已经安装的全局命令。
- 不把 Serena 的所有工具复制成 Codeg Native Rig 工具；Agent 看到的工具来自 MCP `tools/list`。
- 不在第一次打开页面时静默下载；下载由用户勾选并保存后触发，并显示进度。
- 不因为某一个工具缺失而阻塞会话。工具不可用时，该工具隐藏或标记为不可用，其他工具和会话继续运行。

## 3. 现状与主要问题

| 区域 | 当前实现 | 问题 |
| --- | --- | --- |
| 配置 | `CodeIntelConfig` 只有 `codegraph` 与 `lsp` 两组字段 | 无法表达 Serena、安装来源、MCP 命令和版本状态 |
| LSP | `async-lsp` + `LspPool`，Codeg 自己暴露 LSP MCP 门面 | Codeg 维护协议和进程细节，无法直接复用官方 LSP-MCP 配置 |
| CodeGraph | supervisor 启动 `codegraph serve --mcp`，adapter 代理远端工具 | 生命周期与 LSP 逻辑耦合，UI 仍暴露 `.codegraph` 和二进制路径细节 |
| Codeg Agent | `NativeTurnTools` 额外注册 `lsp` 和 `codegraph` | 与 MCP 工具重复，工具描述和外部 Agent 不一致 |
| 页面 | `/settings/code-intelligence` 页面显示 LSP MCP 工具列表、CodeGraph 索引和二进制路径 | 页面名称和操作方式偏工程实现，不像 Agent 单个配置页 |
| 安装 | 页面只显示安装提示，未实现按选项自动安装 | 用户需要自己打开终端安装并重新检查 |

本方案保留现有 MCP 注入、工作区隔离、进程回收和失败开放的基础能力，但把 LSP、CodeGraph、Serena 统一为可登记的外部 MCP Provider。

## 4. 官方启动契约

Codeg 只负责把官方命令转换为 argv 数组，禁止经 shell 拼接。Provider 的命令、参数、环境变量和版本来源都记录在代码内的版本化目录中，运行时只允许使用已登记的配置。

### 4.1 Serena

Serena 官方提供 `start-mcp-server` 子命令。按工作区启动时采用显式项目路径，避免多个工作区共享一个活动项目：

```text
serena start-mcp-server --context=codex --project <workspace>
```

如果系统没有 `serena` 命令，Codeg 使用官方推荐的 uvx 入口作为托管命令：

```text
uvx --from git+https://github.com/oraios/serena serena \
  start-mcp-server --context=codex --project <workspace>
```

生产实现需要把 Serena 版本固定在 Provider manifest 中；示例中的 Git 地址用于说明官方来源，不作为未经评审的动态依赖。默认 context 使用适合编码 Agent 的上下文，具体工具集合通过 Serena 官方 context 和 Codeg 的只读 allowlist 共同控制。Codeg 不调用 `serena setup`，避免修改 Claude、Codex、Cursor 等外部配置。

官方参考：[Serena 客户端配置](https://github.com/oraios/serena/blob/main/docs/02-usage/030_clients.md)、[Serena CLI `start-mcp-server`](https://github.com/oraios/serena/blob/main/src/serena/cli.py)。

### 4.2 CodeGraph

CodeGraph 继续使用官方 MCP 入口：

```text
codegraph serve --mcp
```

Codeg 为每个工作区只启动一个实例，工作目录设置为 workspace。已有全局 `codegraph` 优先使用；缺失时使用 Provider manifest 指定的官方 npm/GitHub 发布包下载到 Codeg 管理目录，再以绝对路径启动。Codeg 不调用 `codegraph install`，不修改其他 Agent 的配置文件。

官方参考：[CodeGraph MCP Server](https://colbymchenry.github.io/codegraph/reference/mcp-server/)。

### 4.3 LSP

LSP 采用“语言 Provider”模型。每种语言由 Provider manifest 声明：

- 语言检测条件：manifest 文件、扩展名和可选项目标识；
- 全局 MCP 命令检测方式；
- 官方安装来源、版本、校验信息；
- MCP 启动命令、参数、工作目录和环境变量；
- 该 Provider 暴露的工具范围。

首次实现只登记有明确官方启动文档和可验证发行包的语言 Provider。现有 `PresetLsp` 表可以作为语言检测和默认勾选的迁移来源，但不能直接把原来的语言服务器 stdio 命令当作 LSP-MCP 命令。没有官方 MCP Provider 的语言，页面显示“未配置官方 MCP”，允许用户保留未勾选状态；自定义 Provider 另列为后续能力。

## 5. 总体架构

### 5.1 组件职责

```text
┌──────────────────────────────┐
│ 设置 → 代码工具               │
│ 总开关                        │
│ LSP / CodeGraph / Serena      │
│ 语言选择、并发上限、状态       │
└──────────────┬───────────────┘
               │ 保存并确保工具可用
               ▼
┌──────────────────────────────┐
│ CodeToolRegistry              │
│ Provider manifest             │
│ 全局发现 → 托管目录 → 下载     │
└──────────────┬───────────────┘
               │ 启动/停止/健康检查
               ▼
┌──────────────────────────────┐
│ ProjectCodeToolsSupervisor    │
│ 按 canonical workspace 去重    │
│ LSP × N、CodeGraph、Serena     │
│ PID、取消、日志、状态           │
└──────────────┬───────────────┘
               │ 官方 MCP transport
               ▼
┌──────────────────────────────┐
│ Codeg project MCP endpoint    │
│ 工具名称来自各 Provider        │
│ 仅过滤冲突/禁止工具             │
└──────────────┬───────────────┘
               │ 会话级连接
      ┌────────┴────────┐
      ▼                 ▼
 外部 ACP Agent      Codeg Agent
```

### 5.2 运行原则

1. **总开关优先。** 总开关关闭时不下载、不启动、不注入任何代码工具。
2. **单项独立。** LSP、CodeGraph、Serena 各自拥有启用状态、安装状态、进程状态和错误信息。一个工具失败不会关闭其他工具。
3. **按工作区隔离。** supervisor 的 key 是 canonical workspace；同一工作区的多个 Agent 共享 MCP 子进程，切换工作区时不复用活动项目。
4. **官方命令直启。** Codeg 负责 argv、cwd、环境、PID 和 transport，不修改官方 MCP 协议。
5. **动态工具面。** `tools/list` 结果按实际已连接的 Provider 合并；未连接的工具不假装可用。工具显示名称、描述和输入 schema 保留官方返回值。
6. **失败开放。** 下载失败、命令缺失、握手失败或工具超时只影响对应 Provider，Agent 可继续使用已有文件工具和其他 MCP。
7. **只读默认。** 代码工具全部使用官方 MCP 的只读能力时跳过权限卡；涉及 Serena 写入、CodeGraph 管理和安装动作的工具不注入会话，由 Codeg UI/宿主专门处理。

## 6. 配置模型

配置仍保存到 `~/.codeg/codeg-agent/code-intel.json`，但从旧的二组模型迁移为三类 Provider。建议结构如下，字段名以实现阶段最终 Rust 类型为准：

```json
{
  "enabled": false,
  "lsp": {
    "enabled": true,
    "languages": ["rust", "go", "python", "typescript"],
    "max_concurrent": 2,
    "auto_install": true,
    "providers": {}
  },
  "codegraph": {
    "enabled": true,
    "auto_install": true,
    "binary_path": null
  },
  "serena": {
    "enabled": false,
    "auto_install": true,
    "command": null,
    "version": null,
    "context": "codex",
    "modes": ["interactive", "editing", "planning"]
  }
}
```

字段语义：

| 字段 | 说明 |
| --- | --- |
| `enabled` | 代码智能总开关；默认 `false` |
| `lsp.enabled` | 是否允许 LSP Provider 接入 |
| `lsp.languages` | 允许自动接入的开发语言；只在工作区检测命中时启动 |
| `lsp.max_concurrent` | 同时运行的 LSP-MCP Provider 上限，默认 `2`，范围 `1..8` |
| `lsp.auto_install` | 选中语言的 Provider 缺失时是否自动下载 |
| `codegraph.enabled` | 是否接入 CodeGraph |
| `codegraph.auto_install` | 缺少官方命令时是否自动下载 |
| `codegraph.binary_path` | 可选的绝对路径；为空时按全局命令和托管目录顺序查找 |
| `serena.enabled` | 是否接入 Serena |
| `serena.auto_install` | 缺少 Serena 命令时是否使用官方 uvx/发布包安装 |
| `serena.command` | 可选自定义命令，仅允许绝对路径或已解析到 PATH 的命令 |
| `serena.context` | 传给 `start-mcp-server` 的官方 context |
| `serena.modes` | 允许的官方 Serena mode；安装和工具过滤不通过 shell 实现 |

配置中不保存进程 PID、临时端口或健康检查结果。运行状态由后端实时生成；历史工具名称只在转录兼容层保留，不继续作为 Native 工具注入。

## 7. 发现、下载与启动流程

### 7.1 统一 Provider 状态机

```text
未启用
  │ 用户勾选并保存
  ▼
检查全局命令/官方 MCP 配置
  ├─ 找到且版本合格 ─────────────┐
  └─ 缺失或版本不合格             │
       │ auto_install=true       │
       ├─ 下载官方发布物并校验      │
       └─ 失败 → 可重试/继续会话   │
                                  ▼
                         生成官方 argv + cwd
                                  │
                           MCP handshake
                         ├─ 成功 → 已连接
                         └─ 失败 → 启动失败
```

### 7.2 发现优先级

1. 用户明确配置的绝对路径。
2. 当前进程通过 `which`/平台等价方式解析到的全局命令。
3. Codeg 管理目录中的已校验版本。
4. 当 Provider 允许自动安装时，下载到 Codeg 管理目录后再次解析。

Codeg 不覆盖第 2 层的文件，也不把第 4 层写回用户的 shell profile。全局命令检测必须展示实际路径和版本，方便用户知道 Codeg 使用的是哪一份工具。

### 7.3 安装安全边界

- 所有进程使用 `Command::new` + `args`，禁止 shell 字符串。
- Provider manifest 固定下载源、版本、SHA-256 或签名校验方式；校验失败删除临时文件。
- 下载写入临时文件，校验成功后原子移动到版本目录。
- 不执行 Provider 的安装脚本，不调用会修改第三方 Agent 配置的 setup/install 子命令。
- Serena、CodeGraph 和 LSP 的工作目录只能是当前 workspace；文件访问仍受 `FileSystemRuntime` 策略约束。
- 删除工具只删除 Codeg 管理目录，不删除用户全局安装。

## 8. LSP 设计

### 8.1 语言选择与启动集合

启动集合定义为：

```text
已勾选语言
∩ 当前 workspace 检测命中
∩ 已找到/已下载官方 LSP-MCP Provider
∩ 当前运行数未超过 max_concurrent
```

勾选语言不会立即为所有工作区启动进程。工作区打开且总开关开启时先做轻量检查；首次需要该语言工具时懒启动。超过并发上限的 Provider 进入等待队列，空闲后启动。切换工作区或取消语言选择时，supervisor 优雅停止不再需要的进程。

### 8.2 页面子选项

- 开发语言多选列表：显示语言、检测结果、Provider 状态和版本。
- “最多同时运行”数字输入，默认 `2`，限制 `1..8`。
- “缺失时自动安装”开关；关闭时只检查并提示安装，不下载。
- “查看启动配置”折叠面板：展示官方命令、参数和来源链接，不允许直接编辑生成的 argv。

### 8.3 与现有实现的关系

目标实现不再使用 `LspPool` 处理 MCP 语义，也不再把 `lsp` 注册到 `NativeTurnTools`。现有语言检测模块可以保留并提取为 Provider 选择器；`async-lsp`、自定义 LSP MCP facade 和 `LspTool` 在迁移完成后删除或降为兼容模块，避免同一语言启动两套服务。

## 9. CodeGraph 设计

- 选中后按官方 `codegraph serve --mcp` 启动；每个 workspace 只保留一个进程。
- 索引初始化、同步和 MCP 启动状态在 Provider 内维护；页面显示“未索引、初始化中、已连接、失败”四种状态。
- `codegraph init/sync` 只能由宿主的后台维护任务调用，不暴露给 Agent 工具。
- `codegraph install/uninstall/ui/web/daemon/upgrade/telemetry/uninit` 永不由 Codeg 代调用。
- `codegraph_explore` 等工具名称直接取官方 `tools/list`；Codeg 只过滤管理类或与内置文件工具冲突的工具。
- CodeGraph 缺失时不再把“安装 npm 包”的说明塞到 Agent 工具结果中；安装动作在页面中显示进度和错误。

## 10. Serena 设计

- 选中后按 workspace 启动一个 Serena MCP server；使用显式 `--project <workspace>`，不依赖 Agent 当前 cwd。
- 默认 context 使用 `codex`，允许在高级设置中选择官方支持的 context；不把“替换其他 Agent 的 MCP 配置”作为安装步骤。
- 默认 mode 组合按 Serena 官方能力选择；编辑 mode 是否开启由 Serena 开关和 Codeg 工具权限策略共同决定。
- 对 Serena 返回的工具做冲突过滤：隐藏重复的 `grep`、目录枚举和基础文件编辑工具，保留符号检索、引用、项目激活和语义编辑能力。过滤清单版本化，避免依赖工具名称模糊匹配。
- Serena 的写入工具仍通过 Codeg Agent/ACP 的 MCP 权限策略执行；工具调用必须写入转录，取消时等待子进程终止或完成清理。

## 11. 「代码工具」页面草图

页面沿用 Agent 单个配置页的纵向布局：页头说明当前作用域，左侧为工具选择与状态，右侧为选中工具详情；移动端改为上下排列。

```text
┌ 设置 ───────────────────────────────────────────────────────────────┐
│ 代码工具                                                            │
│ 管理工作区中的代码分析、代码图谱和 Serena MCP 工具。                 │
│                                                                      │
│ ┌────────────────────────────┐  ┌────────────────────────────────┐ │
│ │ 代码智能                    │  │ 当前工作区                      │ │
│ │ 为 Agent 提供代码工具        │  │ ~/work/codeg                   │ │
│ │                         [开] │  │                                │ │
│ └────────────────────────────┘  └────────────────────────────────┘ │
│                                                                      │
│ ┌────────────── 工具 ─────────┐  ┌──────── Serena ────────────────┐ │
│ │ ● LSP                 已连接 │  │ Serena MCP                 [开] │ │
│ │   Rust · TypeScript          │  │ 语义检索、符号编辑和项目记忆       │ │
│ │                             │  │ 状态：未安装                     │ │
│ │ ○ CodeGraph            未启用│  │ 命令：serena start-mcp-server    │ │
│ │   调用关系与影响分析          │  │ 来源：github.com/oraios/serena   │ │
│ │                             │  │                                │ │
│ │ ○ Serena               未安装│  │ [检查] [安装并启动]              │ │
│ └─────────────────────────────┘  │                                │ │
│                                  │ 高级设置                         │ │
│                                  │ Context  [codex          v]      │ │
│                                  │ Mode     [interactive, editing]  │ │
│                                  │ 缺失时自动安装              [✓]  │ │
│                                  └────────────────────────────────┘ │
│                                                                      │
│ LSP 详情（选择 LSP 时展开）                                          │
│ [✓] Rust   已检测  rust-analyzer MCP   [✓] TypeScript  已检测       │
│ [ ] Go     未检测  gopls MCP           [ ] Python      未配置        │
│ 最多同时运行 [ 2 ] 个 LSP     缺失时自动安装 [✓]                    │
│                                                                      │
│                                               [取消] [保存并应用]     │
└──────────────────────────────────────────────────────────────────────┘
```

页面交互规则：

1. 总开关关闭时，三张工具卡仍显示配置和最近状态，但不能启动，也不触发下载。
2. 勾选单项后，保存按钮显示“保存并应用”；后端按该项执行发现/下载/启动，页面实时显示阶段。
3. 工具已找到但握手失败时保留“重试”和“查看日志”，不把失败改写成“未安装”。
4. LSP 语言行只展示当前 workspace 的检测结果；“未检测”不等于 Provider 不可用。
5. 高级配置默认折叠，避免普通用户接触命令行参数；所有字段旁显示“官方配置”链接。
6. 页面只维护 Codeg 自己的配置，不写入 `mcp.json`、Claude 配置或其他 Agent 配置文件。

## 12. 会话注入与工具冲突

### 12.1 外部 ACP Agent

连接层继续根据 Agent 的 MCP transport 能力注入工作区 MCP endpoint。HTTP-capable Agent 使用项目级 Streamable HTTP；只支持 stdio 的 Agent 使用现有无业务逻辑连接器。连接器只转发 MCP 消息，不启动第二个 Serena、CodeGraph 或 LSP。

### 12.2 Codeg Agent

Codeg Agent 的 `NativeTurnTools` 删除 `LspTool` 与 `CodegraphTool` 两个固定槽位，统一从项目 MCP session 取得动态工具。这样 Codeg Agent 与外部 Agent 看到相同的官方工具名称、描述、参数和可用性。

旧会话或旧转录中的 `lsp` / `codegraph` 工具调用只用于历史展示和分类，不重新执行，也不作为新会话的工具别名。新工具分类由 MCP server name 和 Provider metadata 判断，避免把 Serena 的语义检索误归类为普通网页搜索。

### 12.3 冲突策略

| 冲突 | 处理 |
| --- | --- |
| Serena 与 Codeg `grep/glob/read_file` 重复 | 通过 Serena allowlist 隐藏基础重复工具 |
| CodeGraph 与 Serena 都提供项目搜索 | 两者都可启用；卡片和工具描述标注“关系/影响分析”与“语义检索/编辑”的差异 |
| 外部 Agent 自己已配置同名 Serena | 项目注入前按名称和命令指纹去重；项目级 Codeg 配置优先，不修改外部文件 |
| LSP 多语言 Provider 超出上限 | 排队，不静默杀掉正在服务的 Provider；状态显示等待原因 |
| 旧 native 工具与新 MCP 同时存在 | 迁移版本中禁止旧 native 工具注册，防止重复调用 |

## 13. 配置迁移

读取旧 `code-intel.json` 时执行内存迁移并在用户保存时写回新格式：

| 旧字段 | 新字段 |
| --- | --- |
| `enabled` | `enabled` |
| `lsp.auto_attach` | `lsp.enabled` |
| `lsp.checked` | `lsp.languages`，按现有 preset 的 `language` 映射 |
| `lsp.max_concurrent` | `lsp.max_concurrent` |
| `codegraph.enabled` | `codegraph.enabled` |
| `codegraph.binary_path` | `codegraph.binary_path` |
| `lsp.custom` | 暂存为迁移报告，不自动当作官方 Provider 执行 |

迁移不会自动开启 Serena，也不会因为旧配置中启用了 CodeGraph/LSP 而在读取页面时下载。用户点击保存后才按新配置执行确保流程。迁移失败保留原文件并在页面显示可导出的错误信息。

## 14. 错误与可观测性

每个 Provider 对外提供以下状态字段：

- `configured`：用户是否勾选；
- `discovery`：全局命令、托管目录、未找到；
- `install`：未开始、下载中、已安装、失败；
- `runtime`：未启动、启动中、已连接、退出、握手失败；
- `version`、`resolved_command`、`last_error`、`last_started_at`。

日志至少包含 workspace、Provider id、解析来源、命令 basename、退出码和错误分类；不记录 API Key、完整环境变量和用户文件内容。页面把可恢复错误分为：

1. **缺少运行时**：例如找不到 `uvx`；显示安装指引和重试。
2. **下载失败**：显示来源、网络错误或校验失败；允许重试。
3. **启动失败**：显示退出码、stderr 摘要和日志入口。
4. **MCP 握手失败**：显示 transport、协议错误和官方配置链接。
5. **工具调用失败**：只结束当前调用，不关闭其他 Provider。

## 15. 测试与验收

### 15.1 后端单元测试

- 配置默认值包含总开关和三个独立选项。
- 旧配置迁移后语言、并发数和 CodeGraph 路径保持一致。
- Provider manifest 只生成预期 argv；路径中包含空格时不经 shell 仍能启动。
- 全局命令优先于托管目录；托管目录优先于下载。
- 下载校验失败不会替换已有版本。
- 同一 canonical workspace 复用 supervisor，不同 workspace 不共享 Serena 活动项目。
- 总开关关闭或单项关闭时不启动对应 Provider。
- LSP 语言检测、并发上限和等待队列行为正确。
- 某一 Provider 握手失败时，其他 Provider 和会话仍可用。
- ACP HTTP/stdio 注入和 Codeg Agent 动态工具列表一致。

### 15.2 前端测试

- 导航显示“代码工具”。
- 总开关关闭时三项不可启动；独立开关互不影响。
- LSP 详情正确展示语言检测、最大并发数和自动安装选项。
- 保存按钮提交新配置，安装/启动状态有加载、成功和失败状态。
- 自动安装不在页面初次加载时触发。
- 旧状态字段能以迁移后的三 Provider 状态渲染。
- 远程工作区和桌面工作区使用同一套配置交互。

### 15.3 验收场景

1. 新用户打开页面：总开关关闭，页面不执行下载或启动。
2. 只启用 Serena：Codeg 找到 `serena` 就直接启动；找不到且允许自动安装时下载后启动；LSP 和 CodeGraph 不启动。
3. 启用 CodeGraph 但没有索引：后台初始化，页面显示“初始化中”，Agent 会话可以先建立。
4. 启用 LSP 并勾选 Rust/TypeScript：只有检测到对应语言且 Provider 可用时启动，最多同时运行配置的数量。
5. Serena 退出：页面显示退出原因，CodeGraph/LSP 状态不受影响，点击重试可恢复。
6. 服务器模式通过 HTTP 访问页面：安装和启动状态与桌面模式使用同一后端 API。

## 16. 分阶段实施建议

### P0：配置模型与页面重命名

- 增加三 Provider 配置结构和状态结构。
- 导航及国际化改为“代码工具”。
- 页面按 Agent 单配置页重排，先使用模拟 Provider 状态。
- 加入旧配置迁移和前端/后端契约测试。

### P1：统一 Provider Registry 与 supervisor

- 抽取 Provider manifest、发现、下载校验和启动接口。
- 把现有 CodeGraph 启动逻辑迁移到 Registry。
- 保留项目级 MCP endpoint 和进程回收能力。

### P2：Serena 接入

- 接入 `serena` 全局发现和官方 uvx fallback。
- 按 workspace 显式传 `--project`，完成 context/mode 配置。
- 接入工具 allowlist、状态展示、重试和日志。

### P3：LSP-MCP Provider 接入

- 为已确认官方配置的语言建立 Provider manifest。
- 将语言检测与 `max_concurrent` 接到 Provider supervisor。
- 移除 `async-lsp`、`LspPool`、`LspTool` 和 Codeg 自建 LSP MCP facade。

### P4：Native 工具收口与兼容验证

- 从 `NativeTurnTools`、探索子代理和权限映射中移除 `lsp`/`codegraph` 固定工具。
- 保留旧转录分类和迁移回归测试。
- 完成桌面、服务器、HTTP/stdio ACP Agent 的全链路验收。

## 17. 待评审决策

以下事项需要在实现计划前确认：

1. LSP 首批 Provider 的官方来源、版本和各平台安装包清单。
2. Serena 默认版本固定策略，以及 `uvx` 不可用时是否提供预下载 wheel/二进制缓存。
3. Serena 默认 mode 是否开启 editing；开启后哪些写入工具需要 ACP 权限卡。
4. CodeGraph 索引初始化是否在勾选保存后立即后台执行，还是等首次 Agent 会话。
5. 三类 Provider 是否统一代理成一个 `codeg-code-tools` MCP endpoint，还是按 Provider 分别注入 MCP server descriptor。
6. 是否允许用户新增自定义 LSP-MCP Provider；本方案建议首期只开放官方登记项。
