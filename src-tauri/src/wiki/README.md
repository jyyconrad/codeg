# Personal Wiki

Wiki 使用一份 Markdown 目录保存可独立阅读的笔记，同时用 SQLite 登记来源、项目绑定、处理任务与执行历史。桌面与服务器调用同一套业务实现；输入只记录当前素材路径和来源，Agent 自主整理，代码补齐元数据。

## 业务职责

| 业务 | 主要文件 | 边界与关联 |
| --- | --- | --- |
| 成功轮次采集 | `snapshot.rs`、`filter.rs`、`redact.rs`、`source.rs`、`raw.rs` | 从 ACP 接收有限会话片段，过滤脱敏后登记来源；pending 日志用于防丢失，不固定后续输入版本 |
| 资料归档 | `import.rs`、`session_import.rs` | 上传、粘贴、历史会话和目录导入共用提取/原件存储，返回逐项结果；不调用模型 |
| 后台处理 | `engine.rs`、`worker.rs` | 定时与手动排队、认领、取消和重试；每个任务携带自己的 Wiki 身份 |
| Agent 整理 | `llm.rs`、`turn_summary.rs`、`session_rollup.rs`、`compile.rs`、`synthesis/` | 分别生成单轮记录、对话总结与知识笔记；模型接收素材地址，宿主分配笔记身份并补来源 |
| 输出保存 | `fs_policy.rs`、`commit.rs`、`result.rs` | 校验目标目录，保护人工修改，原子落盘并记录真实产物；不做输入冻结或逐行证据认证 |
| 目录与元数据 | `library.rs`、`project_metadata.rs`、`managed_document.rs` | 代码采集本地文件夹/Git信息并输出可导航的 Markdown；保留用户修改，不改 `.obsidian` |
| 阅读与搜索 | `read_model/`、`tree.rs` | 统一正文/原文读取、目录、搜索、分页、来源存在性及任务产物状态；文件检查不放在数据库服务中 |
| 设置与资源选择 | `settings.rs`、`paths.rs`、`vault.rs`、`lifecycle.rs` | 管理单 Wiki 选择、独立模型配置和初始化；运行状态目录独立于用户笔记目录 |
| 视图刷新 | `events.rs` | 持久化完成后通过 EventEmitter 通知桌面/Web，事件本身不是生成成功证明 |
| 持久化 | `db/service/wiki_service.rs`、`wiki_pipeline_service.rs` | 前者管理来源/项目/任务及重试历史；后者在一个事务内记录批次、来源贡献和素材处理去重 |
| 运行时适配 | `commands/wiki*.rs`、`web/handlers/wiki*.rs` | 参数转换、错误映射和事件发送；普通导入直接调用业务模块，共享设置/任务控制用例保留 core 函数 |
| 前端 | `components/wiki/`、`lib/wiki-api.ts`、`wiki-types.ts`、`wiki-content.ts` | API集中在Wiki模块，WikiData统一URL与刷新代次；列表与目录共用正文阅读和Markdown解析 |

## 必须区分的状态

- 素材地址是关联信息。排队和重试不承诺读取历史固定版本，不要求输入哈希、完整阅读证明或行号证据；长文件由 Agent 按需阅读。
- `result::WikiInput` 是结果与去重记录；输入摘要仅用于避免对相同内容重复归纳，不发给模型作认证。
- 输出提交需要保护人工修改。文件系统与 SQLite 是两个落盘边界，准备好的输出字节和提交记录用于恢复未完成的写入；已完成的提交不能复活用户后来删除的笔记。
- 来源存在性和任务产物可用性在阅读时计算。数据库查询只返回已保存的数据；缺失原始来源不影响阅读已有正文。
- `lifecycle` 锁协调目录选择与任务/来源登记；`commit` 文件锁协调落盘。二者用途不同，都不跨模型执行持有。

## 当前目录与升级边界

正文默认在 `wiki/`，原件、日志与暂存在 `wiki-state/`。优先使用 `CODEG_HOME`、`CODEG_DATA_DIR`，最后回退到 `~/.codeg`；用户可单独指定正文目录，状态目录位置不因此改变。设置只读取 `wiki_settings`，目录标记使用 `.codeg-wiki.json`；不以目录名区分数据版本，模型绑定独立保存。

历史迁移保留执行顺序和已登记的 ID，避免重复执行结构变更；当前代码不会清空 Wiki 数据、配置或自动删除旧表。现有文件直接复制到统一目录，保留同一份数据库身份与关系；旧目录只作为待验证副本保留，用户确认后再删除。不新增多版本切换或后台兼容迁移框架。

本机调整目录时，先暂停旧程序的 Wiki 自动写入，再复制文件、更新资料库地址和唯一配置。旧程序的配置键在验证期间仅用于保持写入暂停；新程序只读取 `wiki_settings`。普通聊天不受影响。新版程序启用后继续使用原有任务和来源，不重新建立项目或来源 ID。

当前查询入口统一为 `wiki_list_jobs_page`、`wiki_list_sources_page` 和 `wiki_read_source_document`；最后一个响应的 `source` 字段包含来源元数据。它们分别替代旧 `wiki_list_jobs`、`wiki_list_sources` 和 `wiki_get_source`。任务详情 `wiki_get_job`、项目绑定查询、来源标注与版本关联接口仍有独立业务用途。

阅读与来源的当前合同见 [Wiki 目录与轻量元数据设计](../../../docs/design/wiki-vault-navigation-metadata.md)。它优先于 [早期 v2 修复设计](../../../docs/design/personal-wiki-repair-design.md) 中已被用户取消的冻结与证据门槛。
