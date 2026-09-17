//! 把本地历史会话或选定目录批量转成 Wiki 资料。
//! 会话由现有解析器读取，目录按格式/大小/数量筛选，最终交给 import 统一归档。
//! 返回逐项成功、失败和重复结果；不额外创建聊天会话或自动启动知识归纳。

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
use crate::wiki::import::{self, ImportFilePart, ImportFilesParams, ImportTextParams};

pub const MAX_SESSION_IMPORTS: usize = 200;
pub const MAX_DIRECTORY_FILES: usize = 100;

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "wiki-state",
    "wiki",
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
    pub partial: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default)]
    pub results: Vec<import::ImportFileResult>,
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
        if path.is_dir() {
            if SKIP_DIR_NAMES.contains(&name.as_ref()) || name.starts_with('.') {
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
        partial: 0,
        errors: Vec::new(),
        results: Vec::new(),
    };
    for (index, (agent, external_id)) in selected.into_iter().enumerate() {
        if index >= MAX_SESSION_IMPORTS {
            result.skipped += 1;
            continue;
        }
        match import_one_session(conn, agent, &external_id).await {
            Ok(one) => record_import(&mut result, one),
            Err(e) => record_file(
                &mut result,
                import_failure(
                    external_id.clone(),
                    session_request_id(agent, &external_id),
                    e.to_string(),
                ),
            ),
        }
    }
    if result.imported + result.duplicates > 0 {
        engine::notify_jobs();
    }
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
    let mut result = import_directory_files(conn, &root, files, &request_id).await;

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
            Ok(one) => record_import(&mut result, one),
            Err(e) => record_file(
                &mut result,
                import_failure(
                    summary.title.clone().unwrap_or_else(|| summary.id.clone()),
                    session_request_id(agent, &summary.id),
                    e.to_string(),
                ),
            ),
        }
    }

    if result.imported + result.duplicates > 0 {
        engine::notify_jobs();
    }
    Ok(result)
}

async fn import_directory_files(
    conn: &DatabaseConnection,
    root: &Path,
    files: Vec<PathBuf>,
    request_id: &str,
) -> WikiBulkImportResult {
    let mut result = WikiBulkImportResult {
        imported: 0,
        duplicates: 0,
        failed: 0,
        skipped: 0,
        partial: 0,
        errors: Vec::new(),
        results: Vec::new(),
    };
    // Each discovered path receives a stable request key even if a sibling
    // cannot be read. Filtering unreadable files must not renumber retry keys.
    for path in files {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document")
            .to_string();
        let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy();
        let file_request = format!(
            "{request_id}:dir:{}",
            crate::wiki::raw::content_hash(&relative)
        );
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                record_file(
                    &mut result,
                    import_failure(filename, file_request, e.to_string()),
                );
                continue;
            }
        };
        let params = ImportFilesParams {
            request_id: file_request.clone(),
            files: vec![ImportFilePart {
                filename: filename.clone(),
                mime: None,
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                original_path: Some(path.to_string_lossy().into_owned()),
            }],
            material_role: Some("reference".into()),
            personal_role: None,
            title: None,
            source_url: Some(path.to_string_lossy().into_owned()),
            author: None,
            batch_id: Some(request_id.to_string()),
            project_ids: None,
            area_ids: None,
        };
        match import::import_files_with_result(conn, params).await {
            Ok(batch) => {
                for item in batch.results {
                    record_file(&mut result, item);
                }
            }
            Err(e) => record_file(
                &mut result,
                import_failure(filename, file_request, e.to_string()),
            ),
        }
    }

    result
}

fn import_failure(filename: String, request_id: String, error: String) -> import::ImportFileResult {
    import::ImportFileResult {
        filename,
        request_id,
        error: Some(error),
        status: "failed".into(),
        duplicate: false,
        source: None,
    }
}

fn record_import(
    result: &mut WikiBulkImportResult,
    one: crate::db::service::wiki_service::WikiImportResult,
) {
    let filename = one
        .source
        .title
        .clone()
        .or_else(|| one.source.original_filename.clone())
        .unwrap_or_else(|| "会话材料".into());
    let request_id = one
        .source
        .request_id
        .clone()
        .unwrap_or_else(|| one.source.id.clone());
    record_file(result, import::file_result(filename, request_id, one));
}

fn record_file(result: &mut WikiBulkImportResult, item: import::ImportFileResult) {
    match item.status.as_str() {
        "failed" => {
            result.failed += 1;
            if result.errors.len() < 8 {
                if let Some(error) = &item.error {
                    result.errors.push(format!("{}: {error}", item.filename));
                }
            }
        }
        "duplicate" => result.duplicates += 1,
        _ => result.imported += 1,
    }
    if item
        .source
        .as_ref()
        .is_some_and(|s| s.extraction_status.as_deref() == Some("partial"))
    {
        result.partial += 1;
    }
    result.results.push(item);
}

async fn import_one_session(
    conn: &DatabaseConnection,
    agent: AgentType,
    external_id: &str,
) -> Result<crate::db::service::wiki_service::WikiImportResult, AppCommandError> {
    let external = external_id.to_string();
    let detail =
        tokio::task::spawn_blocking(move || build_agent_parser(agent).get_conversation(&external))
            .await
            .map_err(|e| AppCommandError::io_error(e.to_string()))?
            .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
    let title = detail
        .summary
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{agent} session"));
    let folder = detail
        .summary
        .folder_path
        .as_deref()
        .map(|path| format!("Folder: {path}\n"))
        .unwrap_or_default();
    let index = format!("# {title}\n\nAgent: {agent}\n{folder}Session: {external_id}\n");
    let req = session_request_id(agent, external_id);
    let imported = import::import_text(
        conn,
        ImportTextParams {
            request_id: req,
            text: index,
            title: Some(title),
            source_url: None,
            author: None,
            material_role: Some("reference".into()),
            personal_role: None,
            project_ids: None,
            area_ids: None,
        },
    )
    .await?;
    let _ = crate::wiki::locator::write_locator(
        &crate::wiki::paths::resolve_state_root(),
        &crate::wiki::locator::SourceLocator {
            source_id: imported.source.id.clone(),
            kind: "local-session".into(),
            conversation_id: None,
            run_id: None,
            agent_type: Some(agent.as_wire().to_string()),
            session_id: Some(external_id.to_string()),
            original_path: None,
            title: imported.source.source_title.clone(),
        },
    );
    Ok(imported)
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

    #[tokio::test]
    async fn directory_results_match_failed_files_and_keep_retry_identity() {
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let root = dir.path().join("materials");
        fs::create_dir_all(&root).unwrap();
        crate::wiki::settings::save_settings(
            &db.conn,
            &crate::wiki::settings::WikiSettings {
                enabled: true,
                vault_path: Some(vault.to_string_lossy().into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let local_home = dir.path().to_string_lossy().to_string();
        temp_env::async_with_vars(
            [
                ("CODEG_HOME", Some(local_home.as_str())),
                ("CODEG_DATA_DIR", None::<&str>),
            ],
            async {
                let ok = root.join("ok.txt");
                let broken = root.join("broken.pdf");
                let empty = root.join("empty.txt");
                let missing = root.join("missing.txt");
                fs::write(&ok, "Readable evidence.").unwrap();
                fs::write(&broken, b"%PDF-1.4\nbroken").unwrap();
                fs::write(&empty, "").unwrap();
                let files = vec![missing.clone(), ok, broken, empty];
                let first =
                    import_directory_files(&db.conn, &root, files.clone(), "directory-result")
                        .await;
                assert_eq!((first.imported, first.failed, first.duplicates), (1, 3, 0));
                assert_eq!(first.results.len(), 4);
                assert_eq!(
                    first
                        .results
                        .iter()
                        .filter(|item| item.status == "failed")
                        .count(),
                    first.failed
                );
                assert!(first
                    .results
                    .iter()
                    .filter(|item| item.status == "failed")
                    .all(|item| item.error.is_some()));
                let original = first
                    .results
                    .iter()
                    .find(|item| item.filename == "ok.txt")
                    .unwrap()
                    .source
                    .as_ref()
                    .unwrap()
                    .id
                    .clone();
                fs::write(&missing, "Newly readable source.").unwrap();
                let second =
                    import_directory_files(&db.conn, &root, files, "directory-result").await;
                assert_eq!(
                    (second.imported, second.failed, second.duplicates),
                    (1, 2, 1)
                );
                assert_eq!(second.results.len(), 4);
                let duplicate = second
                    .results
                    .iter()
                    .find(|item| item.filename == "ok.txt")
                    .unwrap();
                assert_eq!(duplicate.status, "duplicate");
                assert_eq!(duplicate.source.as_ref().unwrap().id, original);
                let added = second
                    .results
                    .iter()
                    .find(|item| item.filename == "missing.txt")
                    .unwrap();
                assert_eq!(added.status, "succeeded");
                assert_ne!(added.source.as_ref().unwrap().id, original);
            },
        )
        .await;
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
    fn discover_skips_application_directories_and_collects_markdown() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("note.md"), "hello").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git").join("HEAD"), "ref").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules").join("x.md"), "skip").unwrap();
        for name in ["wiki", "wiki-state"] {
            fs::create_dir_all(dir.path().join(name)).unwrap();
            fs::write(
                dir.path().join(name).join("generated.md"),
                "skip generated material",
            )
            .unwrap();
        }
        let files = discover_import_files(dir.path(), 20).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("note.md"));
    }
}
