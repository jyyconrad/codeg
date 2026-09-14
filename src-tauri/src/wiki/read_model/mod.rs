//! 个人 Wiki 读模型入口与响应类型：正文、来源、任务、搜索和统计。
//! 桌面命令与 HTTP 适配器共用查询；页面内容取自 Markdown，任务和来源身份取自数据库。
//! 生成与提交属于其他模块；这里仅在使用时补文件存在性、产物状态及展示信息。

pub(crate) mod catalog;
pub mod document;
mod query;

use serde::{Deserialize, Serialize};

pub use query::{
    get_overview, list_jobs, list_notes, list_sources, read_job, read_note, read_source_document,
    read_vault_file,
};

/// 高级文件浏览器读取的原始 Markdown；正文阅读使用 WikiNoteDetail。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiVaultFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiNoteSummary {
    pub note_id: String,
    pub path: String,
    pub title: String,
    pub summary: String,
    #[serde(rename = "type")]
    pub page_type: String,
    pub updated_at: Option<String>,
    pub project_ids: Vec<String>,
    pub source_ids: Vec<String>,
    pub evidence_level: Option<String>,
    pub excerpt: Option<String>,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiSourceReference {
    pub availability: String,
    pub source_url: Option<String>,
    pub source_id: Option<String>,
    pub title: String,
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiNoteDetail {
    pub note: WikiNoteSummary,
    pub body: String,
    pub source: String,
    pub format_warning: bool,
    pub sources: Vec<WikiSourceReference>,
    pub headings: Vec<document::Heading>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiSourceDocument {
    pub read_error: Option<String>,
    pub source: crate::db::service::wiki_service::WikiSourceInfo,
    pub body: String,
    pub raw: String,
    pub format_warning: bool,
    pub related_notes: Vec<WikiNoteSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiOverview {
    pub enabled: bool,
    pub note_count: usize,
    pub source_count: u64,
    pub active_job_count: usize,
    pub pending_memory_count: usize,
    pub failed_job_count: usize,
    pub recent_notes: Vec<WikiNoteSummary>,
    pub next_compile_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WikiNoteQuery {
    pub view: Option<String>,
    #[serde(rename = "type")]
    pub page_type: Option<String>,
    pub project_id: Option<String>,
    pub query: Option<String>,
    pub scope: Option<String>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WikiPage<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

pub(crate) fn page<T>(items: Vec<T>, offset: Option<u64>, limit: Option<u64>) -> WikiPage<T> {
    let total = items.len() as u64;
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(30).clamp(1, 100);
    WikiPage {
        items: items
            .into_iter()
            .skip(offset.min(total) as usize)
            .take(limit as usize)
            .collect(),
        total,
        offset,
        limit,
    }
}
