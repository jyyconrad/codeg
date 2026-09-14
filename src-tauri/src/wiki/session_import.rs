//! Import local agent transcripts and directory documents into the wiki.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::app_error::AppCommandError;
use crate::db::service::import_service;
use crate::document_extract::MAX_NORMALIZED_CHARS;
use crate::models::{AgentType, ContentBlock, ConversationDetail, SelectedSessionKey, TurnRole};
use crate::parsers::build_agent_parser;
use crate::wiki::engine;
use crate::wiki::import::{
    self, ImportFilePart, ImportFilesParams, ImportTextParams, MAX_IMPORT_FILES,
};

pub const MAX_SESSION_IMPORTS: usize = 200;
pub const MAX_DIRECTORY_FILES: usize = 100;

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "wiki-state",
    ".obsidian",
    ".codeg",
];

#[derive(Debug, Clone, Deserialize)]
pub struct ImportLocalSessionsParams {
    pub request_id: String,
    #[serde(default)]
    pub selections: Vec<SelectedSessionKey>,
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportDirectoryParams {
    pub request_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiBulkImportResult {
    pub imported: usize,
    pub duplicates: usize,
    pub failed: usize,
    pub skipped: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

pub fn session_request_id(agent: AgentType, external_id: &str) -> String {
    format!("wiki-session:{}:{external_id}", agent.as_wire())
}

pub fn session_detail_to_markdown(detail: &ConversationDetail) -> String {
    let title = detail
        .summary
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Imported session");
    let mut out = String::new();
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!("Agent: {}\n", detail.summary.agent_type));
    if let Some(path) = detail.summary.folder_path.as_deref() {
        out.push_str(&format!("Folder: {path}\n"));
    }
    if let Some(model) = detail.summary.model.as_deref() {
        out.push_str(&format!("Model: {model}\n"));
    }
    out.push('\n');
    for turn in &detail.turns {
        let heading = match turn.role {
            TurnRole::User => "User",
            TurnRole::Assistant => "Assistant",
            TurnRole::System => "System",
        };
        let body = turn_text(turn);
        if body.trim().is_empty() {
            continue;
        }
        out.push_str(&format!("## {heading}\n\n{body}\n\n"));
    }
    clip_chars(&out, MAX_NORMALIZED_CHARS)
}

fn turn_text(turn: &crate::models::MessageTurn) -> String {
    let mut parts = Vec::new();
    for block in &turn.blocks {
        match block {
            ContentBlock::Text { text } => {
                let t = text.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
            ContentBlock::ToolUse {
                tool_name,
                input_preview,
                ..
            } => {
                let preview = input_preview.as_deref().unwrap_or("").trim();
                if preview.is_empty() {
                    parts.push(format!("Tool: {tool_name}"));
                } else {
                    parts.push(format!("Tool: {tool_name}\n{preview}"));
                }
            }
            ContentBlock::ToolResult {
                output_preview,
                is_error,
                ..
            } => {
                let preview = output_preview.as_deref().unwrap_or("").trim();
                if preview.is_empty() {
                    continue;
                }
                if *is_error {
                    parts.push(format!("Tool error:\n{preview}"));
                } else {
                    parts.push(format!("Tool result:\n{preview}"));
                }
            }
            ContentBlock::Thinking { .. }
            | ContentBlock::Image { .. }
            | ContentBlock::ImageGeneration { .. } => {}
        }
    }
    parts.join("\n\n")
}

fn clip_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(24)).collect();
    s.push_str("\n\n…[truncated]…\n");
    s
}

pub fn discover_import_files(root: &Path, max: usize) -> Result<Vec<PathBuf>, AppCommandError> {
    if !root.is_dir() {
        return Err(AppCommandError::invalid_input("path is not a directory"));
    }
    let mut out = Vec::new();
    walk_dir(root, max, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_dir(dir: &Path, max: usize, out: &mut Vec<PathBuf>) -> Result<(), AppCommandError> {
    if out.len() >= max {
        return Ok(());
    }
    let rd = fs::read_dir(dir).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    for entry in rd {
        if out.len() >= max {
            break;
        }
        let entry = entry.map_err(|e| AppCommandError::io_error(e.to_string()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && SKIP_DIR_NAMES.iter().any(|s| *s == name.as_ref())
            || SKIP_DIR_NAMES.iter().any(|s| *s == name.as_ref())
        {
            if path.is_dir() {
                continue;
            }
        }
        if path.is_dir() {
            if SKIP_DIR_NAMES.iter().any(|s| *s == name.as_ref()) || name.starts_with('.') {
                continue;
            }
            walk_dir(&path, max, out)?;
            continue;
        }
        if is_importable_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_importable_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "md" | "txt" | "pdf" | "docx")
}

fn path_is_under(child: &Path, parent: &Path) -> bool {
    let Ok(child) = child.canonicalize() else {
        return false;
    };
    let Ok(parent) = parent.canonicalize() else {
        return false;
    };
    child.starts_with(parent)
}

pub async fn import_local_sessions(
    conn: &DatabaseConnection,
    params: ImportLocalSessionsParams,
) -> Result<WikiBulkImportResult, AppCommandError> {
    let request_id = params.request_id.trim().to_string();
    if request_id.is_empty() {
        return Err(AppCommandError::invalid_input("request_id is required"));
    }
    let summaries = import_service::collect_local_summaries(|_, _, _, _| {}).await;
    let selected: Vec<(AgentType, String)> = if params.all {
        summaries
            .iter()
            .map(|(at, s)| (*at, s.id.clone()))
            .collect()
    } else {
        params
            .selections
            .iter()
            .map(|s| (s.agent_type, s.external_id.clone()))
            .collect()
    };
    if selected.is_empty() {
        return Err(AppCommandError::invalid_input("no sessions selected"));
    }
    let mut result = WikiBulkImportResult {
        imported: 0,
        duplicates: 0,
        failed: 0,
        skipped: 0,
        compile_job_id: None,
        errors: Vec::new(),
    };
    for (index, (agent, external_id)) in selected.into_iter().enumerate() {
        if index >= MAX_SESSION_IMPORTS {
            result.skipped += 1;
            continue;
        }
        match import_one_session(conn, agent, &external_id).await {
            Ok(ImportOutcome::Created) => result.imported += 1,
            Ok(ImportOutcome::Duplicate) => result.duplicates += 1,
            Err(e) => {
                result.failed += 1;
                if result.errors.len() < 8 {
                    result.errors.push(format!("{agent} {external_id}: {e}"));
                }
            }
        }
    }
    if result.imported + result.duplicates > 0 {
        engine::notify_jobs();
    }
    result.compile_job_id = None;
    Ok(result)
}

pub async fn import_directory(
    conn: &DatabaseConnection,
    params: ImportDirectoryParams,
) -> Result<WikiBulkImportResult, AppCommandError> {
    let request_id = params.request_id.trim().to_string();
    if request_id.is_empty() {
        return Err(AppCommandError::invalid_input("request_id is required"));
    }
    let root = PathBuf::from(params.path.trim());
    if root.as_os_str().is_empty() {
        return Err(AppCommandError::invalid_input("path is required"));
    }
    let files = discover_import_files(&root, MAX_DIRECTORY_FILES)?;
    let mut result = WikiBulkImportResult {
        imported: 0,
        duplicates: 0,
        failed: 0,
        skipped: 0,
        compile_job_id: None,
        errors: Vec::new(),
    };
    for chunk in files.chunks(MAX_IMPORT_FILES) {
        let mut parts = Vec::new();
        for path in chunk {
            match fs::read(path) {
                Ok(bytes) if !bytes.is_empty() => {
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("document")
                        .to_string();
                    parts.push(ImportFilePart {
                        filename,
                        mime: None,
                        bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    });
                }
                Ok(_) => result.skipped += 1,
                Err(e) => {
                    result.failed += 1;
                    if result.errors.len() < 8 {
                        result.errors.push(format!("{}: {e}", path.display()));
                    }
                }
            }
        }
        if parts.is_empty() {
            continue;
        }
        let batch_request = format!(
            "{request_id}:dir:{}",
            result.imported + result.duplicates + result.failed
        );
        match import::import_files_with_result(
            conn,
            ImportFilesParams {
                request_id: batch_request,
                files: parts,
                material_role: Some("unspecified".into()),
                personal_role: None,
                title: None,
                source_url: None,
                author: None,
                batch_id: Some(request_id.clone()),
                project_ids: None,
                area_ids: None,
            },
        )
        .await
        {
            Ok(import::ImportFilesResult::Single(one)) => {
                if one.duplicate {
                    result.duplicates += 1;
                } else {
                    result.imported += 1;
                }
            }
            Ok(import::ImportFilesResult::Batch(batch)) => {
                result.imported += batch.succeeded.saturating_sub(batch.duplicates);
                result.duplicates += batch.duplicates;
                result.failed += batch.failed;
            }
            Err(e) => {
                result.failed += 1;
                if result.errors.len() < 8 {
                    result.errors.push(e.to_string());
                }
            }
        }
    }

    let summaries = import_service::collect_local_summaries(|_, _, _, _| {}).await;
    let mut session_n = 0usize;
    for (agent, summary) in summaries {
        let Some(folder) = summary.folder_path.as_deref() else {
            continue;
        };
        if !path_is_under(Path::new(folder), &root) {
            continue;
        }
        if session_n >= MAX_SESSION_IMPORTS {
            result.skipped += 1;
            continue;
        }
        session_n += 1;
        match import_one_session(conn, agent, &summary.id).await {
            Ok(ImportOutcome::Created) => result.imported += 1,
            Ok(ImportOutcome::Duplicate) => result.duplicates += 1,
            Err(e) => {
                result.failed += 1;
                if result.errors.len() < 8 {
                    result.errors.push(format!("{agent} {}: {e}", summary.id));
                }
            }
        }
    }

    if result.imported + result.duplicates > 0 {
        engine::notify_jobs();
    }
    result.compile_job_id = None;
    Ok(result)
}

enum ImportOutcome {
    Created,
    Duplicate,
}

async fn import_one_session(
    conn: &DatabaseConnection,
    agent: AgentType,
    external_id: &str,
) -> Result<ImportOutcome, AppCommandError> {
    let external = external_id.to_string();
    let detail =
        tokio::task::spawn_blocking(move || build_agent_parser(agent).get_conversation(&external))
            .await
            .map_err(|e| AppCommandError::io_error(e.to_string()))?
            .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let markdown = session_detail_to_markdown(&detail);
    if markdown.trim().is_empty() {
        return Err(AppCommandError::invalid_input("session produced no text"));
    }
    let title = detail
        .summary
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| Some(format!("{agent} session")));
    let req = session_request_id(agent, external_id);
    let imported = import::import_text(
        conn,
        ImportTextParams {
            request_id: req,
            text: markdown,
            title,
            source_url: None,
            author: None,
            material_role: Some("unspecified".into()),
            personal_role: None,
            project_ids: None,
            area_ids: None,
        },
    )
    .await?;
    Ok(if imported.duplicate {
        ImportOutcome::Duplicate
    } else {
        ImportOutcome::Created
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConversationSummary, MessageTurn};
    use chrono::Utc;

    fn turn(role: TurnRole, text: &str) -> MessageTurn {
        MessageTurn {
            id: "t1".into(),
            role,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            timestamp: Utc::now(),
            usage: None,
            duration_ms: None,
            model: None,
            completed_at: None,
            agent_message_id: None,
        }
    }

    #[test]
    fn markdown_keeps_user_and_assistant_text() {
        let detail = ConversationDetail {
            summary: ConversationSummary {
                id: "s1".into(),
                agent_type: AgentType::Grok,
                folder_path: Some("/tmp/switchgear".into()),
                folder_name: Some("switchgear".into()),
                title: Some("方案 C".into()),
                started_at: Utc::now(),
                ended_at: None,
                message_count: 2,
                model: Some("grok-4.6".into()),
                git_branch: None,
                parent_id: None,
                parent_tool_use_id: None,
                delegation_call_id: None,
            },
            turns: vec![
                turn(TurnRole::User, "继续推进工作"),
                turn(TurnRole::Assistant, "按方案 C 分批提交"),
            ],
            session_stats: None,
            transcript_watermark: None,
        };
        let md = session_detail_to_markdown(&detail);
        assert!(md.contains("# 方案 C"));
        assert!(md.contains("## User"));
        assert!(md.contains("继续推进工作"));
        assert!(md.contains("按方案 C 分批提交"));
        assert!(md.contains("Folder: /tmp/switchgear"));
    }

    #[test]
    fn import_does_not_reference_compile_now() {
        // compile_job_id is always None after import; wiki_compile_now is not called.
        let result = WikiBulkImportResult {
            imported: 1,
            duplicates: 0,
            failed: 0,
            skipped: 0,
            compile_job_id: None,
            errors: Vec::new(),
        };
        assert!(result.compile_job_id.is_none());
    }

    #[test]
    fn request_id_is_stable_per_session() {
        assert_eq!(
            session_request_id(AgentType::Grok, "abc"),
            session_request_id(AgentType::Grok, "abc")
        );
        assert_ne!(
            session_request_id(AgentType::Grok, "abc"),
            session_request_id(AgentType::ClaudeCode, "abc")
        );
    }

    #[test]
    fn discover_skips_git_and_collects_markdown() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("note.md"), "hello").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git").join("HEAD"), "ref").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules").join("x.md"), "skip").unwrap();
        let files = discover_import_files(dir.path(), 20).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("note.md"));
    }
}
