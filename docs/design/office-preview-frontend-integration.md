# Office 文件查看预览组件集成方案

| 字段 | 值 |
| --- | --- |
| 文档标题 | Office 文件查看预览组件集成方案 |
| 日期 | 2026-09-15 |
| 状态 | **已废止。** 现行方案见 [统一文件预览](./unified-file-preview.md)（vue-office + SheetJS HTML 表 + 现有图片/HTML/Markdown） |
| 适用版本 | Codeg `0.31.0` 主干（`develop`） |
| 上游调研 | 2026-09-15《浏览器文件预览开源前端库调研》 |
| 事实口径 | **当前实现**以仓库代码为准。**目标**以本文第 2、6、7 节为准。调研附件未做像素级保真度实测，选型后须用本仓库典型附件做门禁 |

本文只回答一件事：在现有文件预览栏里，用哪套开源前端库查看 `.docx` / `.xlsx` / `.pptx`，以及怎样接到现有 `OfficePreview` 上。不改 OfficeCLI 的生成、编辑技能，也不把 PDF、图片、Markdown、CAD 并进同一外壳。

## 1. 当前实现

工作区已经有 Office 预览，入口是只读组件，不走 Monaco。

| 落点 | 行为 |
| --- | --- |
| `src/components/files/office-preview.tsx` | 调用 `start_office_watch` / `stop_office_watch`，用 iframe 指向 `officecli watch` 的本地 HTTP |
| `src/components/files/file-workspace-panel.tsx` | 文件栏遇到 Office 扩展名时无条件走预览（没有源码/预览切换） |
| `src/components/files/file-document-view.tsx` | 会话抽屉、Canvas 文件卡片共用同一组件 |
| `src/lib/language-detect.ts` `isOfficePreviewable` | 仅 `.docx` / `.xlsx` / `.pptx` |
| `src/contexts/workspace-context.tsx` | Office 标签不读文件字节，只放一个空壳；agent 写出文件时可自动开预览 |
| `src/hooks/use-open-file-tabs-watch.ts` | Office 标签不走普通文件热更新，交给 officecli 自己的 SSE |

产品约束已经写进这条路径：

- 桌面窗口直连 `http://127.0.0.1:{port}/`，iframe 保留 `allow-same-origin` 才能接 officecli 的 SSE。
- Web 模式走 `/api/office-watch-proxy/{port}/?cap=…`，去掉 `allow-same-origin`，避免预览页读到应用存储。
- 远程桌面（Tauri 窗口绑远程服务器）因混合内容拦 HTTP iframe，**当前直接提示无法预览**，也不启动 watch。
- 未安装 OfficeCLI 时，预览栏变成安装引导，而不是文档画面。
- 旧的一次性 `officecli_render_html` 会在 agent 写盘时和预览抢 Windows 文件锁；watch 就是为了避开这次重读。

OfficeCLI 仍然承担另一件事：agent 生成和原地改写 Office 文件（设置 → Office 工具、技能包）。查看器和生成器不是同一个子系统。

文案里已有 `officeFullRender` / `officeFullRenderHint`（Morph 动画、3D、公式，会跑幻灯片脚本），组件代码没有引用。可作为「完整渲染」开关的现成文案，不要另起一套说法。

## 2. 目标与非目标

### 2.1 目标

1. 打开工作区里的 `.docx` / `.xlsx` / `.pptx` 时，默认在应用内画出可读预览，不依赖 OfficeCLI 是否已安装。
2. 文件不出域：不把附件交给微软/Google 在线查看器，也不上传到第三方转换 SaaS。
3. 桌面、Web、远程桌面三条宿主都能看（远程桌面走现有 transport 读字节，不再嵌远程 loopback iframe）。
4. agent 连续写同一文件时，预览在写盘稳定后刷新；Windows 上不能因为预览把文件锁死。
5. 对外仍是现在的 `OfficePreview({ rootPath, relPath })`。文件栏、抽屉、Canvas 卡片不各自接一套库。

### 2.2 非目标（本期不做）

- 在预览里编辑、保存、往返改写 OOXML。生成和改写继续走 OfficeCLI 技能。
- `.doc` / `.xls` / `.ppt`、WPS 专有格式、加密算法不是 Agile 的旧加密包。
- 把 PDF、OFD、图片、音视频、代码、压缩包、CAD 塞进同一个「万能查看器」。这些格式已有或应继续分组件（图片、HTML、Markdown、Monaco）。
- Wiki 导入后的原件可视化阅读。Wiki 对 PDF/DOCX 目前只做文本提取。
- 用 ONLYOFFICE / Collabora / kkFileView 在本机再起一套转换服务。

## 3. 用当前项目筛库

调研附件覆盖 PDF、Office、图片、CAD、3D 等。接到 Codeg 上还要过下面这些硬条件。

| 约束 | 来源 | 筛掉什么 |
| --- | --- | --- |
| React 19 + Next.js 16，`output: "export"` | `package.json`，`next.config.ts` | Vue 专用组件；强依赖 Vite 插件且不能拷静态资源的方案 |
| Apache-2.0 发行 | `LICENSE` | AGPL（ONLYOFFICE）、现行商用许可（Handsontable）、源码不得公开传播（`pptx-preview`） |
| 附件是工作区私有路径，不是公开 URL | `read_workspace_file_base64`，office watch proxy | 微软在线 iframe、要求公网 URL 的外壳 |
| 桌面 / Web / 远程桌面 | `office-preview.tsx` | 只能打本机 loopback、或必须再运维转换集群的方案 |
| 二进制标签当前不读文本 | `workspace-context.tsx` | 把 OOXML 当 UTF-8 源码打开的路径 |
| 读字节默认 20 MB、上限 100 MB | `folders.rs` `FILE_BASE64_*` | 默认就把整个文档树胀到数百 MB 且无法收紧的引擎 |
| 中文界面与 CJK 文档 | `src/i18n/messages/zh-CN.json` | 无 CJK 回退、且无法自托管字体的方案 |
| 已有分格式预览栏 | `file-document-view.tsx` | 再引入「273 扩展名全能 SDK」当默认底座 |

用量最大的引擎（PDF.js、docx-preview、exceljs、SheetJS 社区包）可以当底层，但 **docx-preview 是 HTML 近似排版，不是「一页纸」**。产品截图和 README 卖的是工作区内嵌、看起来像文档的预览，HTML 路径只能当保底，不能当主引擎。

## 4. 筛选结论

### 4.1 采用

| 库 | 角色 | 理由 |
| --- | --- | --- |
| **`@silurus/ooxml`**（MIT，调研时 0.87.0） | **默认渲染引擎** | 只管 docx/xlsx/pptx；Rust→WASM 解析 + Canvas；可喂 `ArrayBuffer`，不必公开 URL；官方写明 Next.js Turbopack 可用；有 CJK 区域回退、Worker、渐进排版、查找/选区；`getSelectionContext()` 可接到「把选区发给 agent」 |
| **现有 `officecli watch` iframe** | **可选完整渲染** | 幻灯片脚本、Morph、officecli 自己的实时通道已经存在；未安装或远程桌面不可用时静默退回 Canvas |

### 4.2 备选（不作为默认底座）

| 库 | 何时再评估 |
| --- | --- |
| `@file-viewer/react` + `@file-viewer/preset-office`（Apache-2.0） | 以后要在**同一个外壳**里加 PDF/OFD/ODT，并且接受把 Worker/WASM/字体拷进 `public/`（类似现在的 Monaco `public/vs`）。它按 Vite 插件设计，Next 静态导出要自己跑 `file-viewer-copy-assets`。Office 保真度未用本仓库附件对照过，不能从「273 个扩展名」推断。 |
| `docx-preview` + 表格 HTML | 仅当 `@silurus/ooxml` 在静态导出或 Tauri webview 里 WASM 加载失败、需要无 WASM 保底时。版式会掉页眉页脚和浮动图。不要配 `pptx-preview`（源码传播限制）。 |

### 4.3 明确不用

| 库 | 原因 |
| --- | --- |
| `@cyntler/react-doc-viewer` | 停更；Office 走微软在线 iframe，只要公开 URL；`TXTRenderer` XSS（CVE-2026-30691）未修 |
| `vue-office` | Vue 组件；npm 停更；PPT 底层 `pptx-preview` 源码策略与 Apache-2.0 分发不合 |
| `plangrid/react-file-viewer` | 维护停滞 |
| ONLYOFFICE Document Server | AGPL，是协同编辑服务，不是 npm 查看器 |
| Collabora Online / kkFileView | 转换或在线套件，要独立运维，不是前端组件 |
| Univer | 办公编辑 SDK，作者自己写明不是文件查看器 |
| Handsontable 现行版本 | 非开源商用许可 |
| SheetJS 社区 npm `xlsx@0.18.5` | 停在 2022-03-24，不能当 Excel 预览主引擎 |
| Open File Viewer `0.1.x` | 插件容器仍早期，Office 复杂件还要另接转换 |
| PDF.js / react-pdf / EmbedPDF / Viewer.js / Monaco / model-viewer | 不是 Office。Codeg 已有 Markdown/HTML/图片/代码预览，不必为了「一个库」拆掉 |

File Viewer 的 `preset-lite` / `preset-engineering` / `*-full` 也不进本期：lite 不含 Office 画布级预览，engineering/full 会把 CAD/3D/字体树带进桌面包。

## 5. 推荐形态

**默认 Canvas 查看器，OfficeCLI 降级为可选完整渲染。**

```text
文件栏 / 抽屉 / Canvas 卡片
        │
        ▼
 OfficePreview(rootPath, relPath)          ← 对外 API 不变
        │
        ├─ 默认：OfficeCanvasViewer
        │     读工作区字节 → @silurus/ooxml 按扩展名分发
        │
        └─ 可选：现有 officecli watch iframe
              仅本地桌面且已安装 OfficeCLI
              文案复用 officeFullRender
```

这样同时满足三件事：没装 OfficeCLI 也能看；远程桌面能看；agent 写出来的 Morph/脚本幻灯片仍有一条高保真出口。

不推荐一上来就换 File Viewer 外壳。Codeg 预览栏已经按格式分支，再套一层多格式 SDK 只增加 WASM 体积和资产拷贝，不减少接入面。

## 6. 组件与文件边界

新增文件保持小、可单测；现有 `OfficePreview` 变成路由壳。

| 文件 | 职责 |
| --- | --- |
| `src/components/files/office-preview.tsx` | 选引擎、装失败/过大/加密/远程桌面提示、完整渲染开关。不再在这个文件里画 Canvas。 |
| `src/components/files/office-canvas-viewer.tsx` | 挂载/销毁 silurus viewer，按扩展名分流，处理 zoom 容器高度。 |
| `src/lib/office-canvas-engine.ts` | 无 UI：按扩展名动态载入 `@silurus/ooxml` 的 docx、xlsx、pptx 子路径，`load(ArrayBuffer)`，`destroy()`。测试不依赖真实 WASM。 |
| `src/lib/office-file-bytes.ts` | 用 `read_workspace_file_base64`（有工作区根）或 `read_file_base64`（栏外文件）取字节；base64 → `ArrayBuffer`；把「过大」映射成预览文案。 |
| `src/lib/office-preview-reload.ts` | 根据工作区 `changed_paths` 对当前文件做去抖重载；读失败（含 Windows 共享锁）指数退避，不把错误顶掉上一帧。 |
| `src/components/files/office-watch-frame.tsx` | 从现组件抽出的 iframe + watch 生命周期，只给完整渲染用。 |

调用方继续只引用 `OfficePreview`：

- `file-workspace-panel.tsx`
- `file-document-view.tsx`（抽屉、Canvas 文件节点）

设置页「agent 产出 Office 文件时自动打开预览」(`office-preview-prefs.ts`) 保留。自动打开的是标签，不是 officecli 进程。

## 7. 运行时行为

### 7.1 读文件

Office 标签今天不读字节。接入 Canvas 后，打开标签时读一次完整 OOXML。

- 路径仍是 `(rootPath, relPath)`，走工作区受限接口，不把未校验的绝对路径交给前端拼。
- 默认上限与 `FILE_BASE64_DEFAULT_MAX_BYTES`（20 MB）对齐；设置页或常量允许升到 `FILE_BASE64_MAX_BYTES`（100 MB）。超过则提示文件过大，不尝试解析。
- 读到的是 base64 字符串，解码为 `ArrayBuffer` 再交给 viewer。`load()` 成功后预览组件不再持有文件句柄。
- `~$report.docx` 与隐藏路径继续由现有 `isOfficeOwnerFile` / `isHiddenPath` 拦截，不进入预览。

### 7.2 按格式挂载

官方 API 按子路径拆包，必须按需 import，避免把三种 WASM 打进每个页面。

| 扩展名 | 挂载 | 选项（本期固定） |
| --- | --- | --- |
| `.docx` | `DocxScrollViewer(container)` | `mode: 'worker'`，`progressiveLayout: true`，`enableTextSelection: true`，`cjkFallback` 随界面语言（`zh-CN`→`sc`，`zh-TW`→`tc`，`ja`→`jp`，`ko`→`kr`，其他 `auto`） |
| `.pptx` | `PptxScrollViewer(container)` | 同上；`enableMediaPlayback: false`（完整渲染才跑幻灯片脚本） |
| `.xlsx` | `XlsxViewer(container)` | `mode: 'worker'`；公式只显示缓存值，不在浏览器重算 |

容器必须有确定高度（预览栏已经是 `h-full min-h-0`）。卸载时调用 `destroy()`。同一标签再次 `load()` 由 viewer 替换引擎。

公式（MathJax）、ChartEx、3D 图、TIFF 走官方可选入口，**动态 import，缺省不打进主包**。第一次遇到含公式的文档再拉 `@silurus/ooxml/math`。不要默认 `useGoogleFonts: true`；字体只用来自文档和系统的回退。

### 7.3 热更新（替代 officecli SSE）

agent 写 Office 文件仍会走工作区 `changed_paths`。默认 Canvas 路径要自己接：

1. 只订阅当前预览文件的相对路径。
2. 去抖 400 ms，合并一次 burst。
3. 再读字节；若 Windows 返回共享锁/占用，按 200 ms → 400 ms → 800 ms 重试，最多约 3 s。
4. 成功则 `viewer.load(newBuffer)`；失败保留上一帧，状态条显示「未刷新」。
5. 用户已切到完整渲染时，不走这套重读，避免和 watch 抢锁。

`use-open-file-tabs-watch.ts` 里「Office 跳过普通 etag 刷新」在默认路径上要改：跳过的是**文本 reload**，不是「完全不听变化」。不要把 OOXML 当文本塞进 `tab.content`。

### 7.4 完整渲染开关

仅同时满足：本地桌面、`officecli_detect` 已安装、文件仍是三种 OOXML。打开后行为等于今天的 iframe。远程桌面和 Web 不展示该开关（Web 虽能 proxy，但默认 Canvas 已覆盖查看需求；完整渲染继续维持现有 sandbox 规则即可，不必作为 Web 默认）。

未安装 OfficeCLI 时，预览照常出现，设置入口只用于生成技能，不再挡住查看。

### 7.5 远程桌面

当前实现直接拒绝预览。改成：通过现有 desktop→server transport 调 `read_workspace_file_base64`，在本地 webview 里跑 WASM。文件字节会进这条已鉴权通道，但不进公网查看器。若单文件超过读上限，提示过大，而不是回退到「请去服务器 Web UI」。

### 7.6 选区交给 agent（可后置，接口先留）

silurus 的 `getSelectionContext()` 是只读快照，不暴露可变文档模型。与文件栏 `⌘L` 把选区发给 agent 同类。本期至少：

- 打开文本选区；
- 超链接点击由宿主接管：文档内跳转交给 viewer，`http`/`https`/`mailto`/`tel` 用现有 opener，其余 scheme 丢弃。

把选区插入 composer 可以跟在预览稳定之后，不阻塞查看。

## 8. 构建与体积

项目已经用 `postinstall` 把 Monaco 拷到 `public/vs`。WASM 查看器走同一思路，但 silurus 声明用 `new URL('…', import.meta.url)`，Next 16 Turbopack / webpack 5 可零配置发出 `.wasm`。

落地前必须做一次 **throwaway 探测**（不把探测代码留在主干）：

1. `pnpm add @silurus/ooxml`，在空白客户端页 `load()` 一份最小 docx。
2. `pnpm build` 后检查 `out/_next/static/` 是否包含 `*_parser_bg.wasm` 与 worker。
3. `pnpm tauri` 打开桌面包，确认 webview 能 `WebAssembly.instantiateStreaming` 且 Worker 同源。
4. 静态导出的 MIME：`.wasm` 必须是 `application/wasm`。Tauri 读 `out/` 时若扩展名不对，按 Monaco 方式改为拷到 `public/ooxml/` 并用 `wasmUrl`。

包体策略：

- 三个格式子路径按打开的文件动态 import，不要 `import '@silurus/ooxml'` 全量入口。
- 数学 / ChartEx / 3D / TIFF 继续按需。
- 不要装 `@file-viewer/*-full`。桌面安装包已经含 Monaco 和 sidecar，再叠完整 File Viewer 资产树没有收益。

锁定版本：`package.json` 写死 `@silurus/ooxml` 的精确版本（调研窗口为 `0.87.0`），升级走单独 PR。仓库 README 写明应用代码由 AI 代理生成，按高风险依赖做一次源码与许可证抽查（WASM 依赖应为 MIT/Apache 兼容，无 copyleft）。

## 9. 安全

Canvas 路径不再执行文档内脚本，比今天桌面 iframe `allow-scripts allow-same-origin` 更窄。仍需守这些边界：

| 项 | 规则 |
| --- | --- |
| 读盘 | 只走 `read_workspace_file_base64` / `read_file_base64`，沿用 20–100 MB 上限 |
| 解压 | 调用 silurus `resourceLimits`，单条目和总量不高于读盘上限量级（建议单条目 32 MB、总量 64 MB、条目数 4096），避免 zip bomb 把渲染进程打满 |
| 密码 | Agile 加密包在本地用 `load(bytes, { password })`；密码只留在当前预览会话，不写盘、不上报 |
| 链接 | 宿主 `onHyperlinkClick`；禁止 `javascript:`、`file:`、未识别 scheme |
| XSS | 不把 OOXML 当 HTML `srcDoc` 插进应用源。完整渲染仍用现 iframe sandbox |
| 完整渲染 | Web 模式继续禁止 `allow-same-origin`；capability 不得进 Referer |

不引入 `@cyntler/react-doc-viewer`，也不用任何「把 txt 当 ReactNode」的渲染器。

## 10. 与 OfficeCLI 的分工

| 能力 | 负责方 |
| --- | --- |
| 查看 docx/xlsx/pptx | `@silurus/ooxml`（默认） |
| 幻灯片脚本 / Morph / officecli 专用 HTML | `officecli watch`（可选） |
| agent 创建、改写 Office 文件 | OfficeCLI 技能，不变 |
| 自动打开预览标签 | 现有 `useOfficeAutoPreview` |
| 未安装 OfficeCLI | **不再阻止查看**；设置页仍引导安装，以便生成技能 |

`officecli_render_html` 一次性渲染可以保留给测试或以后的导出，新预览不要再走它（文件锁问题会回来）。

## 11. 测试

沿用 Vitest + Testing Library，不在 CI 里对 WASM 做像素对比。

| 用例 | 期望 |
| --- | --- |
| 默认路径不调用 `startOfficeWatch` | 未开完整渲染时 mock 的 watch API 次数为 0 |
| 按扩展名选择引擎 | `report.docx` → docx 模块，`book.xlsx` → xlsx，`deck.pptx` → pptx |
| 卸载调用 `destroy` | 切走标签后引擎 teardown |
| 文件过大 | 超过默认 20 MB 显示过大提示，不 `load` |
| 读盘占用 | 第一次失败、重试成功后画面更新；耗尽重试则保留旧画面 |
| `changed_paths` 去抖 | 短时间多次变更只 `load` 一次 |
| 远程桌面 | 不再显示 `officeRemoteDesktopUnsupported`，改为读字节 |
| 未安装 OfficeCLI | 默认预览仍尝试 Canvas；完整渲染开关隐藏或不可用 |
| 完整渲染 | 本地桌面 + 已安装时 iframe `src` 仍为 loopback；Web sandbox 不含 `allow-same-origin` |
| owner / hidden | `~$a.docx`、`.git/x.docx` 不进入自动打开 |

真实保真度不进单元测试。合并前用 10–20 份本仓库/本团队的合同、表格、幻灯片做一次人工对照（含中文页眉、表格、图表、嵌入图），把明显掉版记进该 PR，而不是假设「能打开」等于「像 Word」。

## 12. 实施顺序

1. **探测**：静态导出 + Tauri webview 能否加载 silurus 的 wasm/worker。失败则改 `public/ooxml` + `wasmUrl`；仍失败才启用 docx-preview 保底（无 PPT 高保真）。
2. **引擎适配层** + 单测（mock 动态 import）。
3. **字节读取与过大/加密/损坏提示**。
4. **`OfficeCanvasViewer` 接到 `OfficePreview` 默认分支**；三个现有入口应同时亮。
5. **热更新去抖与锁重试**；改 `use-open-file-tabs-watch` 的 Office 例外。
6. **抽出 watch iframe 作为完整渲染**；接上已有 i18n 键。
7. **远程桌面改为读字节**；删掉「无法预览」作为默认文案。
8. **按需 math 模块、链接拦截、资源上限**。
9. **人工保真度门禁**，冻结 `@silurus/ooxml` 版本。

每一步都应能单独编译、单独跑现有 `office-preview.test.tsx` 的演化用例。不要在第一步就把 File Viewer 或 kkFileView 带进来。

## 13. 风险

| 风险 | 处理 |
| --- | --- |
| 复杂域、SmartArt、旧二进制 Office、宏不会像素级还原 | 文档和 UI 明确「预览」；完整渲染给幻灯片脚本。不承诺打印级一致 |
| silurus 代码由 AI 代理编写 | 锁版本；升级单独审 WASM 许可与 changelog；不把该库当编辑器 |
| 重新读盘可能撞上 Windows 文件锁 | 去抖 + 退避；完整渲染仍走 watch；不要回到 `officecli_render_html` |
| Next 静态导出漏拷 wasm | 探测失败则抄 Monaco 的 `public/` 模式 |
| 体积 | 按格式拆包；可选模块按需；拒绝 `preset-all` |
| 调研未实测 | 门禁用真实附件，不靠 star/下载量 |

## 14. 验收

同时满足下列条件，本方案才算落地，而不是「依赖已经写进 package.json」：

1. 未安装 OfficeCLI 的桌面和 Web，能打开工作区内 docx/xlsx/pptx 预览。
2. 远程桌面能打开同一类文件（受 20–100 MB 读上限约束）。
3. 文件栏、会话抽屉、Canvas 文件卡片画面一致，且都不在未开完整渲染时拉起 `officecli watch`。
4. agent 连续保存时预览会更新，Windows 上不出现预览把文件锁死导致技能失败的回归。
5. 完整渲染在本地已安装 OfficeCLI 时仍可用，sandbox 规则不弱于现在。
6. `pnpm test` 覆盖第 11 节用例；`pnpm build` 产物含 wasm 或 `public/ooxml` 回退资产。

Wiki 原件预览、聊天附件气泡、PDF 查看器不在本次验收里。
