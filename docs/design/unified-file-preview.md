# 统一文件预览方案

| 字段 | 值 |
| --- | --- |
| 文档标题 | 统一文件预览方案 |
| 日期 | 2026-09-15 |
| 修订 | 2026-09-15：Excel / CSV 改为 Rust 读盘、按页返回单元格，前端分页渲染；不再使用 SheetJS |
| 状态 | 现行方案，已实现（OfficeCLI 已安装时 docx/xlsx/pptx 仍走 watch；未安装或远程桌面回退到本方案） |
| 适用版本 | Codeg `0.31.0` 主干（`develop`） |
| 替代 | [Office 文件查看预览组件集成方案](./office-preview-frontend-integration.md)（该文以 `@silurus/ooxml` 为默认引擎，不再执行） |
| 事实口径 | **当前实现**以仓库代码为准。**目标**以本文为准 |

用一个前端预览壳，按扩展名分发到指定引擎。PDF / Word / PPT 走 vue-office 同一套内核；Excel / CSV 由 Rust 解析后按页交给前端画表；图片、HTML、Markdown、文本和代码接现有组件。docx / xlsx / pptx 在本机已安装 OfficeCLI 时仍走 `officecli watch`；未安装或远程桌面自动回退到本方案。

## 1. 目标

1. 文件栏、会话抽屉、Canvas 文件卡片走**同一个** `FilePreview`，不再各写一套 `if (office) … if (image) …`。
2. 打开下列类型时，在应用内看到预览，不要求安装 OfficeCLI，也不把文件传到公网查看器：

| 类型 | 引擎 | 来源 |
| --- | --- | --- |
| PDF | vue-office 的 PDF 内核（pdf.js + 虚拟列表） | 新增 |
| Word `.docx` | vue-office 的 Word 内核（docx-preview） | 替换 officecli watch |
| PPT `.pptx` | vue-office 的 PPT 内核（pptx-preview） | 替换 officecli watch |
| Excel `.xlsx` / `.xls` | Rust（calamine）按页返回单元格 JSON | 新增；前端分页表，不把整本工作簿送进浏览器 |
| CSV | Rust（`csv` crate）按页返回单元格 JSON | 新增；与 Excel 共用同一预览组件和分页合同 |
| 图片 | 现有 `ImagePreview`（需求里的 ImageZoomView） | 复用 |
| HTML / HTM | 现有 `HtmlPreview` 沙箱 iframe | 复用 |
| Markdown | 现有 `MarkdownDocumentPreview` | 复用 |
| 文本 / 代码 | 文件栏 Monaco；抽屉/卡片 Shiki `SourceView` | 复用 |

3. OfficeCLI 只保留生成和改写技能。预览进程、`start_office_watch`、远程桌面「无法预览」不再作为默认路径。

## 2. 非目标

- 在预览里编辑、保存、往返写回 OOXML / PDF。
- `.doc` / `.ppt`（OLE 复合文档）。FAQ 写明 vue-office 也不支持这些。
- `@vue-office/excel`、SheetJS、x-data-spreadsheet、Univer。表格不在浏览器里解析，也不在预览里重算公式。
- CAD、3D、压缩包、邮件、OFD。
- Wiki 原件可视化（Wiki 对 PDF/DOCX 仍是文本提取）。
- 把 Vue 运行时打进 Next 应用。

## 3. 当前实现（分发是散的）

| 表面 | 今天怎么选渲染器 |
| --- | --- |
| 文件栏 `file-workspace-panel.tsx` | 本地分支：image → `ImagePreview`；docx/xlsx/pptx → `OfficePreview`（officecli iframe）；HTML 预览开 → `HtmlPreview`；Markdown 预览开 → `MarkdownDocumentPreview`；其余 Monaco |
| 抽屉 / Canvas `file-document-view.tsx` | 同一套分支，代码和文件栏各写一份；只读表面用 Shiki 而不是 Monaco |
| 图片 | `src/components/files/image-preview.tsx`：缩放、百分比、右键拖拽。仓库里**没有**名为 `ImageZoomView` 的组件 |
| HTML | `html-preview.tsx`：资源内联成 `srcDoc`，默认空 `sandbox`（脚本关），用户可开「信任」 |
| Markdown | `markdown-document-preview.tsx` + Streamdown |
| 代码 | 文件栏 Monaco（`public/vs`）；只读表面 `file-document-view.tsx` 的 `SourceView` |
| CSV | `languageFromPath` 落到 `plaintext`，没有表格预览。toolbox `parseCsv` 只给 JSON↔CSV 工具用，不承担文件预览 |
| PDF | 没有预览，当文本打开会坏 |
| Excel | 走 officecli。Rust 侧没有表格预览命令，也没有 calamine |

二进制 Office 标签不读字节，热更新交给 officecli SSE。未安装 CLI 或远程桌面时，预览栏是提示而不是文档。

## 4. 引擎与包名

本仓库是 React 19 + Next 静态导出，不能把 `@vue-office/*` 的 Vue SFC 当成 React 组件来用。vue-office 官方给非 Vue 的入口是同一作者的 `@js-preview/*`（`init(container).preview(data)`），内核与 `@vue-office/pdf|docx|pptx` 相同。

| 需求写法 | 本仓库安装 | 挂载 |
| --- | --- | --- |
| `@vue-office/pdf` | `@js-preview/pdf` | `init(el).preview(ArrayBuffer)` |
| `@vue-office/docx` | `@js-preview/docx` | 同上，并引入对应 `lib/index.css` |
| `@vue-office/pptx` | `pptx-preview`（npm 上没有 `@js-preview/pptx`，该包即 vue-office PPT 内核） | `init(el, { mode: "list" }).preview(ArrayBuffer)` |
| Excel / CSV | 后端 `calamine` + `csv`（实现时按 Cargo 现货锁兼容版本） | `read_spreadsheet_preview` 按页返回字符串单元格 |
| 图片 / HTML / Markdown / 代码 | 不新增前端包 | 现有组件 |

禁止为预览添加 `vue`、`vue-demi`、`@vue-office/excel`、npm `xlsx`。

若构建期发现 `@js-preview/*` 与 `@vue-office/*` 版本对不齐，以能 `init().preview()` 的那套为准，并在 PR 里写清实际锁的版本。不引入第二种 Word/PPT 引擎。

`@vue-office/pptx` / `@js-preview/pptx` 底层 `pptx-preview` 允许自用和商用，但完整源码需向作者索取、不得上网发布。本仓库只装 npm 包，不把该源码拷进 git、不二次分发其未公开实现。

## 5. 统一组件

对外只暴露一个只读预览组件。文件栏在「预览模式」和二进制文件时用它；抽屉和 Canvas 卡片始终用它（它们本来就不能编辑）。

```ts
export type FilePreviewKind =
  | "pdf"
  | "docx"
  | "pptx"
  | "spreadsheet"
  | "image"
  | "html"
  | "markdown"
  | "source"

export function previewKindFromPath(path: string): FilePreviewKind
```

| `previewKindFromPath` | 扩展名 |
| --- | --- |
| `pdf` | `.pdf` |
| `docx` | `.docx` |
| `pptx` | `.pptx` |
| `spreadsheet` | `.xlsx` `.xls` `.csv` |
| `image` | 现有 `IMAGE_EXTENSIONS` |
| `html` | `.html` `.htm` |
| `markdown` | `.md` `.markdown` `.mdx`（mdx 仍走 Markdown 预览，与今天 `language === "markdown"` 一致的部分保持；`.mdx` 今日映射为 `mdx`，预览开关只给 `language === "markdown"`。本期 **不扩大** 到 mdx，避免和现开关行为打架） |
| `source` | 其余 |

```ts
export function FilePreview(props: {
  path: string
  /** 文本类标签已读入的内容；二进制种类忽略 */
  content: string
  rootPath: string | null
  relPath: string | null
  language: string
  /** markdown / html / csv：true 显示渲染，false 显示源码。二进制种类忽略，永远预览 */
  isPreview: boolean
  loading?: boolean
  openFilePreview?: (path: string) => void
})
```

内部按 `previewKindFromPath(path)` 分发，每种一个适配器，适配器之间不互相 import。

| 适配器 | 实现 |
| --- | --- |
| `OfficeJsPreview` | pdf / docx / pptx 共用：动态 import 对应 `@js-preview/*`，在 `ref` 容器上 `init` → `preview(bytes)` → 卸载 `destroy` |
| `SpreadsheetPreview` | 调 `read_spreadsheet_preview`，用返回的 `header` + `rows` 画表；工具栏切 sheet、翻页 |
| `ImagePreview` | 原文件，不改名。需求中的 ImageZoomView 即此组件 |
| `HtmlPreview` | 原文件 |
| `MarkdownDocumentPreview` | 原文件 |
| `SourceView` / Monaco | 不放进 `FilePreview` 的二进制分支。文件栏 `source` 且未开预览时仍走现有 Monaco 列；抽屉/卡片的 `source` 走现有 `SourceView` |

文件栏和 `FileDocumentView` 都改成：能预览的种类交给 `FilePreview`，否则维持今天的编辑器/源码。禁止第三处再抄一份扩展名表。

`~$*.docx` 和隐藏路径继续由 `isOfficeOwnerFile` / `isHiddenPath` 拦截，不进入自动打开。

## 6. 各类型行为

### 6.1 PDF / Word / PPT

1. 打开标签时**不**把文件当 UTF-8 读进 `tab.content`（与今天 image/office 相同：只读预览壳）。
2. 适配器按 `(rootPath, relPath)` 调 `read_workspace_file_base64`（无工作区根则 `read_file_base64`），上限与现网一致：默认 20 MB，硬顶 100 MB。
3. base64 → `ArrayBuffer`，交给 `previewer.preview(buffer)`。不要传公网 URL，也不要把工作区路径塞进 `<img src="file://…">`。
4. 容器必须有确定高度（预览栏已是 `h-full min-h-0`）。
5. 卸载或换文件时 `destroy()`，再 `init` 下一个。
6. pdf.js worker：与 Monaco 一样，构建时拷到 `public/`（路径以实际包内 worker 文件名为准），初始化时把 worker 源指到同源 URL。禁止运行时打 pdf.js CDN。
7. 渲染失败（损坏、加密、超限）显示现有风格的居中提示 + 重试，不回退 officecli。

agent 写同一文件时：订阅 `changed_paths`，对当前路径去抖 400 ms 后重新读字节并 `preview()`。Windows 共享锁按 200 ms / 400 ms / 800 ms 重试，最多约 3 s；失败保留上一帧。不要调用 `officecli_render_html`（会重新抢锁）。

`use-open-file-tabs-watch.ts` 里「Office 跳过刷新」改为：跳过的是**文本 etag reload**，二进制预览由适配器自己听路径变化。

远程桌面：不再显示 `officeRemoteDesktopUnsupported`。字节走现有已鉴权 transport，WASM/Worker 在本地 webview 跑。

### 6.2 Excel / CSV（Rust 读盘 + 前端分页）

整本工作簿不进浏览器。后端打开文件，只返回当前页的单元格字符串；前端画表并翻页。这不是 Excel 的「打印分页」，而是按行（和列窗口）切片。

#### 命令

`read_spreadsheet_preview`。Tauri 与 Web `POST /read_spreadsheet_preview` 共用同一实现。路径规则与 `read_file_preview` 相同：`path` 相对 `rootPath`，走 `resolve_tree_path` + `ensure_user_navigable_path`，在 `run_file_io` 里同步解析，避免卡住异步运行时。

请求（camelCase）：

```ts
type SpreadsheetPreviewQuery = {
  rootPath: string
  path: string
  /** 缺省为第一张表。CSV 只有一张，忽略或固定为文件名 */
  sheet?: string | null
  /** 0-based，相对整张表第 1 行之后的数据行；表头单独返回 */
  rowOffset: number
  /** 默认 100，最大 500，最小 1 */
  rowLimit: number
  /** 0-based 列窗口 */
  colOffset: number
  /** 默认 64，最大 128，最小 1 */
  colLimit: number
}
```

响应：

```ts
type SpreadsheetSheetInfo = {
  name: string
  /** 含表头 */
  rowCount: number
  columnCount: number
}

type SpreadsheetPreviewPage = {
  path: string
  sheets: SpreadsheetSheetInfo[]
  sheet: string
  /** 表第 1 行，已按列窗口切开。CSV 同样把第 1 行当表头 */
  header: string[]
  rowOffset: number
  rowLimit: number
  colOffset: number
  colLimit: number
  /** 含表头在内的总行数 */
  totalRows: number
  totalColumns: number
  /** 数据行，不含 header。每行长度与当前列窗口一致，缺格补 "" */
  rows: string[][]
  eof: boolean
}
```

翻页时 `rows` 从 `rowOffset + 1` 起取（跳过表头行）。`rowOffset = 0` 的第一页数据是表的第 2 行起。只有一行的表：`header` 有值，`rows` 为空，`totalRows = 1`。空表：`header` / `rows` 皆空，`totalRows = 0`。

`eof` 为 true 表示这一页已经含最后一行数据（或没有数据行）。

#### 解析

| 格式 | 库 | 行为 |
| --- | --- | --- |
| `.xlsx` / `.xls` | calamine | 只读；公式用缓存值，不在服务端重算。`.xlsb` / `.ods` 本期不开放 |
| `.csv` | `csv` crate | RFC 4180 引号；不把数字/日期推断成类型，单元格保持原文 |

CSV 编码：先按 UTF-8（接受 BOM）。非法 UTF-8 再试 GB18030。两种都失败则返回可展示的错误，不要静默丢字符。不在前端用 toolbox `parseCsv` 做预览。

单元格显示：

- 空 → `""`
- 文本 → 原文（CSV 前导零必须在）
- 整数 → 十进制、无科学计数
- 小数 → 去掉无意义的尾零，不用浏览器默认 `toString` 的长精度噪声
- 日期/时间 → `YYYY-MM-DD` 或 `YYYY-MM-DD HH:MM:SS`（Excel 序列由 calamine 转）
- 布尔 → `TRUE` / `FALSE`
- 错误值 → `#DIV/0!` 这类原文；未知则 `#ERROR!`

`sheet` 对不上任何表名 → `invalid_input`。`rowOffset` 超出数据行 → 空 `rows` 且 `eof=true`，不是 not_found。`rowLimit` / `colLimit` 越界由服务端 clamp 到本节范围。

文件上限：与 `FILE_OPEN_HARD_LIMIT`（50 MB）对齐。超过则失败，不读出一页算成功。xlsx 解压沿用 zip 炸弹意识：单条目和总量不得高于该上限量级。解析超时 15 s，超时当错误。

Windows 占用：读失败按 200 ms / 400 ms / 800 ms 重试，与 PDF/Word 预览同一套退避；耗尽则保留上一页。

本期不在进程里缓存整表。CSV 流式跳过 `rowOffset` 行，翻页便宜。xlsx 每次请求会再打开工作簿；若实测翻页明显卡，可按 `(规范化路径, mtime, size)` 做短 TTL 缓存，不作为第一版合同。

Wiki 的 `document_extract` 把 xlsx 标成不支持，专管文本抽取。表格预览是新模块，不要塞进 extract，也不要塞进已经过大的 `folders.rs`。

#### 前端

`SpreadsheetPreview`：

- 顶栏：sheet 名（xlsx 多表；CSV 只有一张时不画切换）、列窗口必要时的左右翻列、每页 50 / 100 / 200、上一页 / 下一页。行号按表内 1-based 计，例如 201 行的表在第一页（100 条数据）显示「第 2–101 行 / 共 201 行」。
- 表：`<table>` 画在应用 DOM 里。`header` 用 `th` 吸顶。单元格当文本节点，**禁止** `dangerouslySetInnerHTML`，也不走 `HtmlPreview` iframe。
- 换 sheet 或文件变更时 `rowOffset` / `colOffset` 归零。
- 请求中的 `rowLimit` / `colLimit` 与顶栏选择一致；不要一次把 `totalRows` 全拉下来。
- 工作区 `changed_paths` 命中当前文件时，去抖后按**当前** sheet / 偏移再拉一页，不要退回第一页（除非该 sheet 已不存在）。

`.xlsx` / `.xls` 仍是二进制标签，无源码视图。CSV 仍是文本，文件栏保留源码 ↔ 预览；预览走本命令，源码走 Monaco / `SourceView`，不把整表 JSON 写进 `tab.content`。

### 6.3 图片（ImageZoomView）

继续用 `ImagePreview`。标签仍是 `language: "image"`，内容仍是 `data:` URL。缩放、百分比、右键拖拽、适合窗口，行为不变。统一壳只做分发，不重写灯箱。

`ImagePreviewDialog`（聊天缩略图点击）不是文件栏预览，不并进 `FilePreview`。

### 6.4 HTML

继续 `HtmlPreview`：内联子资源、默认不执行脚本、显式「信任」才 `allow-scripts`。预览开关保持。不把 HTML 改成 vue-office 或裸 `src="http://…"`。

### 6.5 Markdown / 文本 / 代码

- Markdown：文件栏预览开 → `MarkdownDocumentPreview`；关 → Monaco。抽屉/卡片预览开同样走 Markdown，关走 `SourceView`。
- 文本/代码：不新增查看器。文件栏 Monaco，只读表面 Shiki，超过 `SOURCE_VIEW_MAX_BYTES` / `SOURCE_VIEW_MAX_LINES` 的提示保留。
- 不把 `.json` / `.yaml` / `.ts` 改成「默认预览」。它们已经是编辑器主路径。

## 7. 标签加载与开关

`workspace-context` 打开文件时：

| 种类 | `tab.content` | `readonly` | 预览开关 |
| --- | --- | --- | --- |
| pdf / docx / pptx / xlsx / xls | 空，不读文本 | 是 | 无，永远 `FilePreview` |
| image | `data:` URL（现逻辑） | 是 | 无 |
| csv | 文本 | 否 | 有，默认预览 |
| html / markdown | 文本（现逻辑） | 否 | 有（现逻辑，默认是否预览不改） |
| 其它 | 文本 | 否 | 无 |

`isOfficePreviewable` 改为只覆盖 docx/pptx，或拆成 `isBinaryPreviewable`：pdf + docx + pptx + xlsx + xls。xlsx 不再和 docx 共用 officecli 语言 `"office"`。建议标签 `language`：

- pdf → `"pdf"`
- docx / pptx → 可继续 `"office"`，或分成 `"docx"` / `"pptx"`；分发必须以路径为准，不要再只认 `"office"`
- xlsx / xls → `"spreadsheet"`
- csv → `languageFromPath` 增 `csv: "plaintext"` 或 `"csv"`；预览种类看扩展名

自动打开 agent 产出的 Office 文件：仍用 `useOfficeAutoPreview`，范围扩到 pdf（若 agent 写出 pdf）。不启动 watch 进程。

`FileWorkspaceHeader` 的眼睛按钮：在现有 markdown/html 上加上 csv。二进制种类不出现源码切换。

## 8. 文件边界

| 文件 | 职责 |
| --- | --- |
| `src/lib/file-preview-kind.ts` | `previewKindFromPath`、是否二进制、是否有源码/预览切换。单测覆盖扩展名表 |
| `src/components/files/file-preview.tsx` | 统一壳：loading/error 框 + 按 kind 分发 |
| `src/components/files/office-js-preview.tsx` | pdf/docx/pptx 的 js-preview 生命周期 |
| `src/components/files/spreadsheet-preview.tsx` | 调 Rust 分页接口并画表 |
| `src/lib/spreadsheet-preview.ts` | `readSpreadsheetPreview` 的 query 归一（limit clamp）与页码换算，纯函数单测 |
| `src/lib/office-file-bytes.ts` | pdf/docx/pptx 读 base64、超限错误、去抖重载 |
| `src-tauri/src/spreadsheet_preview/mod.rs` | 打开文件、选表、切页、单元格格式化。CSV 与 Excel 分模块 |
| `src-tauri/src/commands/spreadsheet_preview.rs` | `read_spreadsheet_preview` 命令；路径校验复用 folders 的 resolve |
| `src-tauri/src/web/handlers/files.rs` + `router.rs` | Web 暴露同一命令 |
| `src/lib/api.ts` / `src/lib/types.ts` | 前端类型与 transport 调用 |
| `src/components/files/file-document-view.tsx` | 改为调用 `FilePreview` |
| `src/components/files/file-workspace-panel.tsx` | 预览分支改为调用 `FilePreview`；Monaco 分支不动 |
| `src/components/files/office-preview.tsx` | 删除或收成对 `OfficeJsPreview` 的薄包装，避免第三入口 |
| `src/lib/language-detect.ts` | 增加 pdf / spreadsheet / csv 判定；修正 `isOfficePreviewable` 调用点 |
| `src/contexts/workspace-context.tsx` | 二进制种类扩展到 pdf/xlsx/xls；不再为预览假定 officecli |
| `src/hooks/use-open-file-tabs-watch.ts` | 二进制和表格预览走适配器重载，不走文本 etag |
| `src-tauri/Cargo.toml` | `calamine`、`csv`、`encoding_rs` |
| `package.json` | 增加 `@js-preview/pdf`、`@js-preview/docx`、`@js-preview/pptx`；**不加** `xlsx`；postinstall 如需拷 pdf worker |

不把 `FilePreview` 做成可编辑控件，也不把 Monaco 搬进去。

## 9. 安全

| 项 | 规则 |
| --- | --- |
| 读盘 | pdf/docx/pptx 走现有 base64（20–100 MB）。表格走 `read_spreadsheet_preview`，文件上限 50 MB，响应只含当前页 |
| 文件不出域 | js-preview 喂 `ArrayBuffer`；表格只返回单元格字符串。禁止微软/Google 在线 iframe |
| PDF worker | 只加载同源 `public/` 资源 |
| Word/PPT HTML | 渲染在应用页的容器里（库的既有行为）。不给这些容器 `allow-scripts` 的 iframe 特权；文档内超链接若冒泡，只允许 http/https/mailto/tel 或应用内打开文件 |
| Excel/CSV | JSON 单元格在 React 里当文本渲染。不把后端字符串当 HTML 插入 |
| 密码 PDF / 加密 OOXML | 当失败提示，本期不做密码框（vue-office 对加密支持不稳定，不在验收里承诺） |
| 完整渲染 | 不保留 officecli iframe 作为默认或隐藏开关，除非产品后续单独加回 |

今天桌面 officecli iframe 带 `allow-scripts allow-same-origin`。切到前端预览后，这条面缩小。不要为了「更像 PPT」再把脚本打开。

## 10. 与 OfficeCLI 的关系

| 能力 | 负责方 |
| --- | --- |
| 查看 pdf / csv / .xls | `FilePreview`（从不走 watch） |
| 查看 docx / xlsx / pptx | 已安装 OfficeCLI 且非远程桌面 → `officecli watch`；否则 `FilePreview` |
| 查看图片/HTML/Markdown/代码 | 现有组件，经同一壳分发 |
| agent 创建、改写 Office 文件 | OfficeCLI 技能，设置页保留 |
| 未安装 OfficeCLI | **不阻止查看**，自动回退前端引擎 |
| 远程桌面 | 不走 watch iframe，不显示「无法实时预览」；走 `FilePreview` |

设置里「产出 Office 文件时自动打开预览」仍然有效，打开的是 `FilePreview` 标签。

## 11. 测试

Vitest 不跑 pdf.js 像素对比，不加载真实 WASM 也可以把分发测完（动态 import mock 掉）。

| 用例 | 期望 |
| --- | --- |
| 扩展名表 | pdf/docx/pptx/xlsx/xls/csv/png/html/md/ts 各落到正确 kind |
| 统一壳 | 文件栏与 `FileDocumentView` 对同一 path 调同一组件 |
| 默认不 watch | 打开 docx 时 `startOfficeWatch` 次数为 0 |
| 卸载 | 切走 pdf/docx/pptx 调用 `destroy` |
| 过大 | pdf/docx/pptx 超过默认 20 MB、表格超过 50 MB 提示，不解析 |
| 表格分页 | 201 行 CSV（1 表头 + 200 数据）在 `rowLimit=100` 时：`rowOffset=0` 返回 100 条数据且 `eof=false`；`rowOffset=100` 返回 100 条且 `eof=true` |
| 表格列窗口 | `colOffset=64, colLimit=64` 只返回第 65–128 列 |
| CSV 前导零 | `"001"` 进表后仍是 `001` |
| CSV 编码 | GB18030 的中文 CSV 能出正确汉字 |
| 公式 | xlsx 含 `=1+1` 且缓存为 `2` 时预览显示 `2`，不在服务端重算 |
| 远程桌面 | 不再渲染 `officeRemoteDesktopUnsupported` |
| 未安装 CLI | 仍尝试前端预览 |
| owner 文件 | `~$a.docx` 不自动打开 |
| 回归 | markdown/html 开关、图片缩放、Monaco 编辑、抽屉过大源码提示 |

人工：各准备 1–2 个本团队的 pdf、docx、pptx、xlsx、csv，确认能打开。不承诺与桌面 Office 像素一致。docx-preview 会掉复杂页眉和浮动图，这是选定引擎的已知代价。

## 12. 实施顺序

1. `previewKindFromPath` + 单测，不接 UI。
2. `FilePreview` 先只分发到**现有** Image / HTML / Markdown / Source，文件栏和抽屉改为走它。行为应与现在一致。
3. `OfficeJsPreview`：docx → pptx → pdf。每加一种，officecli 对该扩展名退出。
4. pdf worker 拷贝进 `public/`，静态导出和 Tauri 窗口各打开一次最小 pdf。
5. Rust `read_spreadsheet_preview` + 页切片单测（CSV 夹具；xlsx 用最小合法 OOXML 或测试生成的文件）。
6. `SpreadsheetPreview` 接该命令：sheet、行分页、列窗口。xlsx 从 officecli 退出。
7. 二进制与表格热更新去抖；远程桌面去掉拒绝文案；清理 `OfficePreview` 的 watch 调用。
8. 人工抽样后锁 js-preview 与 calamine 版本。

每一步都要能单独跑现有文件预览测试。不要在第 2 步之前装 vue-office 全家桶。

## 13. 验收

1. 未安装 OfficeCLI 时，桌面和 Web 能预览工作区内的 pdf、docx、pptx、xlsx、csv。
2. 远程桌面能预览同一组类型（受读字节上限约束）。
3. 图片缩放、HTML 沙箱、Markdown 预览、代码编辑/高亮与改前一致。
4. 文件栏、抽屉、Canvas 卡片对同一文件种类画面一致，且未开的格式不会误打 officecli watch。
5. Excel/CSV 由 Rust 分页返回；翻页不把整表下载到前端；CSV 前导零保留；单元格不当 HTML 插入。
6. `pnpm test` 与 `cargo test` 覆盖第 11 节自动化用例；`pnpm build` 产物含 pdf worker（或文档中记录的 `public/` 回退）。

不把「像 Word 一样的版式」或「PPT 动画」列入验收。
