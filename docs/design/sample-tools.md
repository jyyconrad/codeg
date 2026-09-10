# 万能小工具 · 功能清单

建议做成**浏览器 / 桌面端本地处理优先的实用工具箱**，围绕文本、开发辅助、编解码、加解密和图片处理展开。工具共享输入、预览、复制、下载组件。

调研日期 **2026-09-10**。清单按实现边界分成两期，版本号只用内部 **v1**、**v2**（不对外发版号）：

| 版本 | 划分标准 |
|---|---|
| **v1** | **纯前端可实现**：Web Crypto、按需 JS 库、Canvas、Worker。不新增 Tauri command |
| **v2** | **必须 Rust 配合**：大文件流式读盘、bcrypt、X.509 证书解析等前端不合适或不可靠的能力 |

多数小工具本身很薄（统计、编解码、JSON、时间戳），全部放进 v1，不再按「好不好写」拆第三期。

## 1. 产品入口

侧栏固定导航当前顺序为：新建会话 → 自动化 → 待办任务 → 仓库面板 → 无限会话 → 文件夹。计划在 **无限会话** 与 **文件夹** 之间新增一行：

| 顺序 | 现有 / 新增 | 说明 |
|---|---|---|
| 1 | 新建会话 | 保持不变 |
| 2 | 自动化 | 保持不变 |
| 3 | 待办任务 | 保持不变 |
| 4 | 仓库面板 | 保持不变 |
| 5 | 无限会话 | 保持不变 |
| **6** | **万能小工具** | **新增。独立 workbench 路由，占满主内容区** |
| 7 | 文件夹 | 会话列表，保持不变 |

行为与现有侧栏路由一致：

- 点击进入独立页面，不占用会话 Tab。
- 可在侧栏「显示选项」中隐藏该行；隐藏后仍可从快捷操作进入。
- 默认可见。状态不写入 URL，刷新回到会话工作区（与无限会话相同）。
- 建议路由 id：`toolbox`。接入点：`WorkbenchRouteId`、`SIDEBAR_NAV_ITEM_IDS`、`sidebar.tsx` 中 canvas 按钮之后。

页面结构：

```
┌ 侧栏 万能小工具 ─┐  ┌ 工具箱主区 ──────────────────────────────┐
│ 搜索工具          │  │ 面包屑：万能小工具 > AES / SM4 加解密     │
│ 收藏 / 最近       │  │ 输入 → 参数 → 处理 → 结果                │
│ 文本              │  │ 示例 · 清空 · 复制 · 下载 · 传到下一工具  │
│ 开发              │  │                                          │
│ 编解码            │  │                                          │
│ 加解密            │  │                                          │
│ 图片              │  │                                          │
│ 通用              │  └──────────────────────────────────────────┘
└──────────────────┘
```

首页为可搜索的分类卡片墙；点进具体工具后，左侧分类保留，右侧换成该工具工作台。

## 2. 调研结论

### 2.1 可参考的产品

| 产品 | 已核实的特点 | 可借鉴的方向 |
|---|---|---|
| [IT Tools](https://github.com/CorentinTh/it-tools) | 面向开发者，自部署；Crypto 分类含 Hash、HMAC、bcrypt、AES 加解密、RSA 密钥对、BIP39、密码强度 | 分类导航、工具模块化、统一页面结构 |
| [Ctool](https://github.com/baiy/Ctool) | 国内开发者常用离线工具箱；哈希含 SM3，加解密含 AES / DES / SM2 / SM4；编解码、JSON、时间戳、二维码齐 | **国密进 v1**、编解码与加解密并列、中文场景（简繁、拼音、命名风格） |
| [Squoosh](https://squoosh.app/) | 图片压缩、本地处理、效果对比 | 文件不上传、参数实时预览、压缩前后对比 |
| [CipherKit](https://cipherkit.app/) | 浏览器本地 AES / RSA / Hash / HMAC / bcrypt / JWT | 加解密参数页（算法、模式、填充、输入输出编码）一次给齐 |
| 土薯 / LZL 等国内在线站 | AES 对齐 OpenSSL / Java 常见组合；SM2 / SM3 / SM4 独立页 | 密钥/IV 支持 UTF-8、Hex、Base64；模式与填充显式可选 |

IT Tools 的 Crypto 分类实际包含：Token 生成、哈希、bcrypt、UUID、ULID、Encrypt/Decrypt（AES 等）、BIP39、HMAC、RSA 密钥对、密码强度、PDF 签名检查。Ctool 把 **SM2 / SM3 / SM4** 和 AES 放在同一套加解密页，这是国内开发者对接政务、金融、能源接口时的刚需，原清单未覆盖。

### 2.2 原清单缺口（加解密）

原稿已有 Base64、哈希、JWT 查看、随机密码，**没有真正的加解密**：

| 缺口 | 日常场景 | 版本 |
|---|---|---|
| AES 加解密 | 对接 Java `AES/CBC/PKCS5Padding`、OpenSSL `enc`、前端 crypto-js 密文 | v1 |
| HMAC | 开放平台签名、Webhook 校验 | v1 |
| 国密 SM3 / SM4 | 国内接口报文加密与摘要 | v1（SM3 并入哈希） |
| 国密 SM2 | 公钥加密、签名验签 | v1（`sm-crypto-v2`） |
| RSA 密钥 / 加解密 / 签名 | PEM 互操作、JWT RS256、接口加签 | v1（Web Crypto） |
| bcrypt | 校验库里的口令哈希 | v2（Rust，避免 JS 卡死） |
| 证书 / PEM 查看 | 看过期时间、SAN、指纹 | v2（X.509 用 Rust 解析） |
| 大文件哈希 / 文件加解密 | 安装包校验、日志/备份加密 | v2（Rust 流式读盘） |

另外，用户常把 **编码当成加密**（搜「Base64 加密」「MD5 加密」）。产品上要把 **编解码** 与 **加解密** 分成两类，同时给编码/哈希工具加上这些检索别名。

### 2.3 明确不做

| 不做 | 原因 |
|---|---|
| CTF 古典密码、RSA 攻击、哈希爆破 | 与编码助手场景不符，且易被滥用 |
| 用友 / 致远 / 海康等厂商私有解密 | 法律与维护成本都不合适 |
| 云端代加密、密钥托管、HSM | 违背「本地处理、密钥不离开本机」 |
| BIP39 / 钱包助记词 | 高风险，浏览器/渲染进程不适合保管资产熵 |
| 密码学教学沙盘（可视化 S-Box、逐步加密） | 超出实用工具定位 |

本工具箱是**调试与转换便利**，不能替代 KMS、证书体系或专业密码机。页面页脚对 ECB、MD5、DES 给出「仅兼容旧系统」提示。OCR、Office 转换、PDF 合并不进入本清单。

## 3. 设计原则

1. **本地优先**：明文、密钥、口令默认不写磁盘、不上传。收藏只记工具 id 与非敏感参数（算法、模式），不记密钥。
2. **参数一次给齐**：加解密页必须显式选择算法、模式、填充、密钥编码、IV、输入/输出编码；禁止隐式默认导致「和 Java 对不上」。
3. **编码 ≠ 加密**：导航分开；结果区需要时提供「将结果送到 AES 解密 / JSON 格式化」。
4. **按工具懒加载**：打开某项再下载 `crypto-js` / `sm-crypto` / 图片编码器。
5. **安全随机**：UUID、密码、IV、盐使用 `crypto.getRandomValues`，禁止 `Math.random`。
6. **能前端就算前端**：v1 不预埋 Tauri command；只有前端内存、性能或 ASN.1 可靠性不够时才进 v2。

## 4. 产品壳（所有工具共用）

| 能力 | 说明 | 版本 |
|---|---|---|
| 工具注册 | 每项登记 `id、名称、分类、关键词/别名、入口、处理位置`，生成导航和搜索索引 | v1 |
| 搜索 | 侧栏顶部即时过滤；匹配名称、别名（如「md5加密」「国密」） | v1 |
| 收藏 / 最近 | 本地记住常用工具；原始文本和文件默认不持久化 | v1 |
| 页面模板 | 「输入 → 参数 → 处理 → 结果」；示例、清空、复制、下载、错误定位 | v1 |
| 计算与 UI 分离 | 将处理函数放入 Worker；正则、图片、大文本可取消 | v1 |
| 文件护栏 | 校验类型、体积、图片像素；批量限制并发，结束后释放内存 | v1 |
| 结果串联 | 例如 Base64 解码 → AES 解密 → JSON 格式化 | v1 |
| 快捷键 | 全局搜索工具、复制结果 | v1 |
| 本地文件任务 | 选文件 → Rust 流式处理 → 进度 → 另存；不上传网络 | v2 |

## 5. 完整工具清单

版本列只标 **v1** / **v2**。同一工具若文本路径与文件路径实现不同，写成 `v1（文本）/ v2（文件）`。处理位置：v1 为前端本地，v2 为 Tauri/Rust 本地（仍不联网）。

### 5.1 文本

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `text-stats` | 字数与文本统计 | 字符、中文字符、词、行、字节；空白是否计入可切换 | v1 |
| `text-clean` | 文本清洗 | 去首尾空格、去空行、统一换行、全半角、中英文标点 | v1 |
| `line-dedupe-sort` | 行去重与排序 | 按行切分，大小写/空白规则生成去重键，排序或保持原序 | v1 |
| `case-naming` | 命名风格转换 | `camelCase` / `snake_case` / `PascalCase` / `kebab-case` / `CONSTANT_CASE` | v1 |
| `text-diff` | 文本差异对比 | 两段文本按行 diff，再标出行内增删 | v1 |
| `find-replace` | 批量查找替换 | 普通字符串与正则；先展示命中和替换预览 | v1 |
| `markdown-preview` | Markdown 预览 | 解析为 HTML，清理危险内容，再渲染、复制或导出 | v1 |
| `zh-convert` | 简繁转换 | 简体 ↔ 繁体（OpenCC 词库级，不只单字） | v1 |

### 5.2 开发

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `json-format` | JSON 格式化与校验 | 缩进或压缩；失败时定位行列；大整数不丢精度（字符串化选项） | v1 |
| `timestamp` | 时间戳转换 | 秒/毫秒 ↔ 日期；显式时区；范围检查 | v1 |
| `uuid` | UUID 生成 | 安全随机 UUID v4；批量生成和复制；可选 v7 | v1 |
| `json-yaml` | JSON ↔ YAML | 双向转换，保留基本类型；失败时指出路径 | v1 |
| `json-csv` | JSON ↔ CSV | 对象数组映射表头；反向时显式选择字段类型 | v1 |
| `regex-tester` | 正则表达式测试 | 实时匹配组与替换；Worker 执行，支持超时终止 | v1 |
| `cron` | Cron 解释 | 五段/六段语法 + 时区；列出未来若干次执行时间 | v1 |
| `sql-format` | SQL 格式化 | 美化 / 压缩；不执行语句 | v1 |
| `code-format` | XML / YAML 查看 | 缩进、折叠、语法错误位置 | v1 |

### 5.3 编解码

与加解密分开。结果区标注「这是编码，不是加密」。

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `base64` | Base64 编解码 | 文本按 UTF-8 转字节再编码；文件 Base64；校验非法字符。别名：base64加密 | v1 |
| `url-codec` | URL 编解码与拆解 | 区分完整 URL 与参数值；展示协议、域名、路径、查询参数 | v1 |
| `hex-string` | Hex ↔ 字符串 | UTF-8 / Latin-1；空格与 `0x` 前缀容忍 | v1 |
| `html-entities` | HTML 实体 | 转义 / 反转义 | v1 |
| `unicode-escape` | Unicode 转义 | `\uXXXX` ↔ 中文；JSON 风格转义 | v1 |
| `radix` | 进制转换 | 2–36 进制整数；大整数用字符串，避免精度丢失 | v1 |

### 5.4 加解密（新增重点）

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `symmetric-cipher` | 对称加密 / 解密 | **AES-128/192/256**、**SM4**；模式 CBC / GCM / ECB（ECB 警告）；填充 PKCS7 / None / Zero；密钥与 IV 可选 UTF-8 / Hex / Base64；明文/密文 Hex 或 Base64。兼容 Java `AES/CBC/PKCS5Padding` 与常见 SM4 报文 | v1（文本）/ v2（文件） |
| `hash` | 哈希与校验和 | 文本或文件；**MD5、SHA-1、SHA-256、SHA-384、SHA-512、SHA3-256、SM3**。别名：md5加密。MD5/SHA-1 标注「仅校验，不作口令」 | v1（文本/小文件）/ v2（大文件） |
| `hmac` | HMAC 签名 | 密钥 + 消息；SHA-256 / SHA-512 / SM3；输出 Hex / Base64 | v1 |
| `password-gen` | 随机密码生成 | 安全随机，指定字符集与长度；避免取模偏差；**不保存结果** | v1 |
| `asymmetric-cipher` | 非对称加密 / 签名 | **RSA**（OAEP / PKCS#1 v1.5）与 **SM2**（C1C3C2 / C1C2C3，可选 ASN.1）；密钥生成（RSA 2048/4096，SM2）；PEM 导入导出；加密解密与签名验签分开展示 | v1 |
| `jwt` | JWT 查看与验签 | 拆解 Header / Payload；Base64URL 解码；可选 HMAC/RSA 验签。解码与验签结果分开，避免「能看就等于合法」 | v1 |
| `password-strength` | 密码强度 | 长度、字符类、估算熵；不上传、不保存 | v1 |
| `totp` | TOTP 一次性口令 | 本地根据 Base32 密钥生成当前码；倒计时。密钥仅留在内存 | v1 |
| `bcrypt` | 口令哈希 | bcrypt 哈希与校验；可调 cost；不把明文写入本地记录 | v2 |
| `cert-pem` | 证书 / PEM 查看 | 解析 PEM 证书：主题、SAN、有效期、指纹；不校 CRL/OCSP | v2 |

对称加密建议做成**一个工具页、算法下拉**，而不是 AES、SM4 各开一页，避免参数组合在两处漂移。非对称同理（RSA / SM2 切换）。v2 的文件能力挂在同一页上，不另开工具。

### 5.5 图片

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `image-compress` | 图片压缩与格式转换 | 解码后调尺寸或质量，导出并对比大小和观感 | v1 |
| `color-convert` | 颜色转换 | HEX、RGB、HSL 互转；校验范围；展示色块和透明度 | v1 |
| `image-crop` | 图片裁剪与缩放 | 选框映射原图坐标，重采样导出；锁定比例 | v1 |
| `image-watermark` | 图片水印 | 文字或图片按透明度、角度、间距叠加 | v1 |
| `image-stitch` | 图片拼接 | 横向、纵向或网格计算位置，绘制到画布 | v1 |

图片走 Canvas，设像素与体积上限。超出上限不在本期用 Rust 重做，直接拒绝并提示。

### 5.6 通用

| id | 工具 | 功能要点 | 版本 |
|---|---|---|---|
| `qr-generate` | 二维码生成 | 文本或链接；纠错级别；导出 PNG / SVG | v1 |
| `qr-decode` | 二维码识别 | 本地读图解码；**不自动打开**结果里的链接 | v1 |
| `unit-convert` | 单位换算 | 同类单位先换成基准再转换；温度单独处理偏移 | v1 |
| `date-diff` | 日期间隔 | 自然日差与精确时长；是否包含起止日 | v1 |

### 5.7 数量与分期

| 分类 | 工具数 | 纯 v1 | v1 + v2 文件能力 | 纯 v2 |
|---|---|---|---|---|
| 文本 | 8 | 8 | 0 | 0 |
| 开发 | 9 | 9 | 0 | 0 |
| 编解码 | 6 | 6 | 0 | 0 |
| 加解密 | 10 | 6 | 2（对称、哈希） | 2（bcrypt、证书） |
| 图片 | 5 | 5 | 0 | 0 |
| 通用 | 4 | 4 | 0 | 0 |
| **合计** | **42** | **38** | **2** | **2** |

v1 交付 40 个可点进的工具页（含对称/哈希的文本路径）；v2 给哈希和对称补文件通道，并新增 bcrypt、证书两个工具页。

## 6. 加解密工具规格

### 6.1 对称加密 / 解密

| 项 | v1（前端，文本） | v2（Rust，文件） |
|---|---|---|
| 算法 | AES-128/192/256，SM4-128；DES / 3DES 可折叠并警告 | 同左，对文件流式加解密 |
| 模式 | CBC、GCM、ECB | 同左 |
| 填充 | PKCS7（Java PKCS5 与此相同）、ZeroPadding、NoPadding | 同左 |
| 密钥输入 | UTF-8 字符串 / Hex / Base64；长度不符时拒绝并提示期望字节数 | 同左；可选 PBKDF2 从口令派生，迭代次数显式 |
| IV / Nonce | 可选手动；「生成随机 IV」；GCM 默认 12 字节 | 同左 |
| 明文 / 密文 | UTF-8 文本，或 Hex / Base64 字节 | 从磁盘流式读写，体积上限单独设 |
| 输出 | Hex 或 Base64；可选把 IV 前置到密文（需标明格式） | 另存文件；可选 OpenSSL `Salted__` 头兼容 |

失败时区分：密钥长度错误、IV 长度错误、密文不是合法 Base64/Hex、GCM 鉴权失败、填充损坏。不要只显示「解密失败」。

### 6.2 哈希

v1：同时计算多种算法，避免用户来回切换。文本与小文件用 Web Crypto / `sm-crypto-v2`，小文件可 `File.stream()` 分块。SM3 与 SHA-256 并列展示。

v2：大文件由 Rust 按块读盘计算，进度可取消；算法集合与 v1 对齐（含 SM3）。

### 6.3 HMAC（v1）

输入：消息、密钥、摘要算法、输出编码。用于核对开放平台 `sign`、GitHub webhook 等。密钥按字节处理，不要先当 UTF-8 再 silently 截断。

### 6.4 非对称（v1）

| 算法 | 能力 | 格式注意 |
|---|---|---|
| RSA | 生成 2048/4096；加密解密；签名验签 | Web Crypto；PEM PKCS#1 / PKCS#8；OAEP 与 PKCS#1 v1.5 分开展示，后者标「仅兼容」 |
| SM2 | 生成密钥对；加密解密；签名验签 | `sm-crypto-v2`；明文 Hex 密钥；C1C3C2（新）/ C1C2C3（旧）；可选 ASN.1；公钥是否带 `04` 未压缩前缀要可切换 |

私钥默认不可导出到「最近使用」。用户主动点「复制私钥」才进剪贴板。

### 6.5 v2 专用

| 工具 | Rust 侧要点 |
|---|---|
| `bcrypt` | 哈希与校验走 Tauri command；cost 默认 10，设上限；耗时在后台，UI 可取消 |
| `cert-pem` | 用 `x509-parser` / `rustls-pemfile` 解析 PEM：主题、SAN、有效期、指纹；不访问网络校 CRL/OCSP |

### 6.6 安全与文案

- 所有加解密、哈希、HMAC、bcrypt、TOTP **仅在本机计算**。
- 不把密钥、口令、TOTP secret 写入 localStorage / 会话记录。
- ECB、MD5、SHA-1、DES、PKCS#1 v1.5 加密给出短警告。
- 页内一句说明：本工具用于联调与格式转换，生产密钥请用专门的密钥管理系统。

## 7. 实现逻辑

**v1（纯前端）**

| 模块 | 建议 |
|---|---|
| AES-GCM、SHA-2、HMAC-SHA、RSA-OAEP、PBKDF2、随机数 | [Web Crypto API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Crypto_API) |
| AES-CBC/ECB + PKCS7（对齐 Java / crypto-js / OpenSSL 常见联调） | 按需加载 crypto-js 或等效实现；**不要假设 Web Crypto CBC 与 Java 默认填充一致就省略参数** |
| SM2 / SM3 / SM4 | [sm-crypto-v2](https://www.npmjs.com/package/sm-crypto-v2)。SM2 密钥格式必须有对照用例 |
| JWT | Base64URL 拆解 + Web Crypto 验签 |
| TOTP / 密码强度 / 简繁 / diff / 正则 | 按需 JS 库 + Worker |
| 图片、二维码 | [Canvas API](https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API)、qrcode / jsQR |
| 耗时任务 | [Web Workers](https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Using_web_workers)，提供取消 |

**v2（Rust / Tauri command，仍不联网）**

| 模块 | 建议 |
|---|---|
| 大文件哈希 | `sha2` + `md-5` + 国密 crate，按块读 `std::fs`，回传进度 |
| 文件对称加解密 | AES-GCM / CBC、SM4；与 v1 参数字段对齐，避免两套密文格式 |
| bcrypt | `bcrypt` crate |
| 证书 | `x509-parser`、`rustls-pemfile` |

v1 不调用这些 command。同一工具页在 v2 落地后，按输入类型分流：文本走前端，大文件走 Rust。

## 8. 版本规划

只分 **v1**、**v2**。版本号仅内部使用。

**v1 · 纯前端**

- 壳：侧栏入口、搜索、分类、收藏、最近、统一模板、复制/下载、结果串联、快捷键。
- 文本 8、开发 9、编解码 6、图片 5、通用 4，全部交付。
- 加解密文本路径：对称（AES+SM4）、哈希（含 SM3）、HMAC、随机密码、RSA/SM2、JWT、密码强度、TOTP。
- 不新增 `src-tauri` command。国密走 `sm-crypto-v2`，RSA / SHA / AES-GCM 走 Web Crypto。

**v2 · 需要 Rust**

- 哈希、对称加密补**文件通道**（流式读盘、进度、另存）。
- 新增 bcrypt、证书 / PEM 查看。
- 本地文件任务进度条（选文件 → command → 进度 → 另存）。
- 与 v1 同一套参数和输出编码，只换执行位置。

## 9. 关键决策

1. **入口叫「万能小工具」，放在无限会话下面**，独立路由，不塞进设置，也不占用会话列表。
2. **编解码与加解密分栏**，避免 Base64/MD5 继续被当成加密；搜索仍响应「base64加密」等别名。
3. **国密进 v1**：SM4 与 AES 同页，SM3 与 SHA 同页，SM2 与 RSA 同页。前端用 `sm-crypto-v2`，不把国密整类推迟到 Rust。
4. **对称加密做成单页多算法**，用参数对齐 Java / OpenSSL，而不是只提供「现代」AES-GCM。
5. **分期只按实现边界**：v1 纯前端，v2 才加 Tauri/Rust；多数小工具很薄，全部进 v1。
6. **密钥与明文不落盘**；收藏只保存工具与非敏感参数。
7. **不做 CTF、爆破、厂商私有算法、助记词钱包**。
8. **bcrypt 和大文件、X.509 才下沉 Rust**；仍然不引入网络服务器。

## 10. 接入现有代码的落点（实现时）

仅作索引，本期文档不改代码：

| 点 | 文件 |
|---|---|
| 路由 id | `src/contexts/workbench-route-context.tsx` 的 `WorkbenchRouteId` |
| 侧栏显示开关 | `src/lib/sidebar-view-mode-storage.ts` 的 `SIDEBAR_NAV_ITEM_IDS` |
| 导航按钮 | `src/components/layout/sidebar.tsx`，画在 canvas（无限会话）之后 |
| 主区挂载 | `src/components/workbench/workbench-content.tsx` 的 `WORKBENCH_ROUTES` |
| 文案 | `src/i18n/messages/*.json` 的 `Folder.sidebar` |
| 工具箱模块（v1） | 新建如 `src/components/toolbox/`（注册表 + 页面模板 + 各工具） |
| Rust 能力（v2） | `src-tauri` 新增 command：文件哈希、文件加解密、bcrypt、证书解析 |
