//! 个人 Wiki 业务入口：连接素材采集、Agent 整理、文件存储与目录阅读。
//! commands/ 和 web/handlers/ 适配桌面与服务器；本目录共用业务实现。
//! 素材只登记路径和来源，阅读时检查存在性；写入保护不承担输入冻结或事实验证。

pub mod commit;
pub mod compile;
pub mod engine;
pub mod events;
pub mod filter;
pub mod fs_policy;
pub mod import;
pub mod library;
pub mod lifecycle;
pub mod llm;
pub mod locator;
pub mod managed_document;
pub mod paths;
pub mod project_metadata;
pub mod prompts;
pub mod raw;
pub mod read_model;
pub mod redact;
pub mod relocate;
pub mod result;
pub mod session_import;
pub mod session_rollup;
pub mod settings;
pub mod snapshot;
pub mod source;
pub mod tree;
pub mod turn_summary;
pub mod vault;
pub mod worker;
