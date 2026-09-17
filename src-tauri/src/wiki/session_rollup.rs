//! 在对话明确完成后生成work/sessions中的会话总结。
//! 优先提供已有轮次笔记，必要时使用本地会话导出的Markdown；与turn_summary
//! 共用正文包装和来源关联规则，经commit写入，保持与原始归档相互独立。

use std::fs;
use std::path::Path;

use chrono::Utc;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::entities::conversation;
use crate::db::entities::conversation::{ConversationKind, ConversationStatus};
use crate::db::entities::{wiki_job, wiki_source};
use crate::db::error::DbError;
use crate::db::service::{folder_service, wiki_service};
use crate::models::{AgentType, ConversationDetail};
use crate::wiki::commit::{self, StagedProposal};
use crate::wiki::compile::{check_leaf_body, yaml_string, CompileError};
use crate::wiki::llm::{WikiLlm, WikiLlmError, WIKI_SESSION_ROLLUP_MAX_TURNS};
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::raw;
use crate::wiki::result::{JobOutputManifest, WikiOutput};
use crate::wiki::settings;
use crate::wiki::turn_summary::{self, MemoryPageFields};
use crate::wiki::vault;
use crate::wiki::worker::WorkerError;

pub const KIND: &str = "session_rollup";
pub const PAGE_TYPE: &str = "session-summary";

pub fn page_rel(conversation_id: i32) -> String {
    format!("work/sessions/c{conversation_id}.md")
}

pub fn dedupe_key(conversation_id: i32) -> String {
    format!("session_rollup:{conversation_id}")
}

pub fn conversation_id_from_job(job: &wiki_job::Model) -> Option<i32> {
    let manifest: Value = serde_json::from_str(job.input_manifest.as_deref()?).ok()?;
    i32::try_from(manifest.get("conversation_id")?.as_i64()?).ok()
}

/// Thin hook from `conversation_service`. No-ops when wiki is off or capture
/// exclusions apply.
pub async fn enqueue_on_completed(conn: &DatabaseConnection, conversation_id: i32) {
    if let Err(e) = enqueue_on_completed_inner(conn, conversation_id).await {
        tracing::warn!(
            conversation_id,
            error = %e,
            "[wiki] session_rollup enqueue after Completed failed"
        );
    }
}

async fn enqueue_on_completed_inner(
    conn: &DatabaseConnection,
    conversation_id: i32,
) -> Result<(), DbError> {
    let _transition = crate::wiki::lifecycle::lock().await;
    crate::wiki::relocate::relocate_legacy_app_data_wiki(conn).await?;
    let settings = settings::load_settings(conn).await?;
    if !settings.enabled {
        return Ok(());
    }
    let Some(conv) = conversation::Entity::find_by_id(conversation_id)
        .one(conn)
        .await?
    else {
        return Ok(());
    };
    if conv.status != ConversationStatus::Completed || conv.deleted_at.is_some() {
        return Ok(());
    }
    match conv.kind {
        ConversationKind::Delegate | ConversationKind::Loop => return Ok(()),
        _ => {}
    }
    if settings
        .capture
        .exclude_agent_types
        .iter()
        .any(|t| t == &conv.agent_type)
    {
        return Ok(());
    }
    let mut root_folder_id = Some(conv.folder_id);
    let mut folder_path = None;
    if let Ok(Some(folder)) = folder_service::get_folder_by_id(conn, conv.folder_id).await {
        folder_path = Some(folder.path.clone());
        root_folder_id = Some(folder.parent_id.unwrap_or(folder.id));
        let excluded = &settings.capture.exclude_folder_ids;
        if excluded.contains(&folder.id)
            || excluded.contains(&folder.parent_id.unwrap_or(folder.id))
        {
            return Ok(());
        }
    }

    let vault = resolve_vault_path(settings.vault_path.as_deref());
    vault::initialize_vault(&vault).map_err(DbError::from)?;
    vault::initialize_state_root(&resolve_state_root()).map_err(DbError::from)?;
    let canonical = vault.to_string_lossy().to_string();
    let vault_row = wiki_service::ensure_active_vault(conn, &canonical).await?;
    let _ = settings::ensure_db_instance_id(conn).await?;

    let sources =
        wiki_service::list_sources_for_conversation(conn, &vault_row.id, conversation_id).await?;
    let has_acp = sources.iter().any(|s| s.source_kind == "acp-turn");
    if !has_acp {
        ensure_local_session_raw(
            conn,
            &vault_row.id,
            &vault,
            &conv,
            folder_path.as_deref(),
            root_folder_id,
        )
        .await?;
    }

    enqueue_session_job(conn, &vault_row.id, conversation_id).await?;
    crate::wiki::engine::notify_jobs();
    Ok(())
}

pub async fn enqueue_session_job(
    conn: &DatabaseConnection,
    vault_id: &str,
    conversation_id: i32,
) -> Result<wiki_job::Model, DbError> {
    let key = dedupe_key(conversation_id);
    let rel = page_rel(conversation_id);
    let manifest = json!({
        "conversation_id": conversation_id,
        "rel": rel,
    })
    .to_string();
    wiki_service::insert_kind_job(
        conn,
        vault_id,
        KIND,
        &key,
        None,
        Some(&manifest),
        wiki_service::InsertMode::ActiveOnly,
    )
    .await
}

async fn ensure_local_session_raw(
    conn: &DatabaseConnection,
    vault_id: &str,
    vault: &Path,
    conv: &crate::db::entities::conversation::Model,
    folder_path: Option<&str>,
    root_folder_id: Option<i32>,
) -> Result<String, DbError> {
    if let Some(existing) = wiki_service::find_local_session_source(conn, vault_id, conv.id).await?
    {
        return Ok(existing.id);
    }
    let body = export_conversation_transcript(conv)?;
    if body.trim().is_empty() {
        return Err(DbError::Validation(
            "local-session export produced no text".into(),
        ));
    }
    let source_id = uuid::Uuid::new_v4().to_string();
    let hash = raw::content_hash(&body);
    crate::wiki::locator::write_locator(
        &crate::wiki::paths::resolve_state_root(),
        &crate::wiki::locator::SourceLocator {
            source_id: source_id.clone(),
            kind: "local-session".into(),
            conversation_id: Some(conv.id),
            run_id: None,
            agent_type: Some(conv.agent_type.clone()),
            session_id: conv.external_id.clone(),
            original_path: None,
            title: conv.title.clone(),
        },
    )
    .map_err(DbError::from)?;
    let _ = vault;
    let _ = folder_path;
    let now = Utc::now();
    let db_instance_id = crate::wiki::settings::ensure_db_instance_id(conn).await?;
    let project_ids = if let Some(root_id) = root_folder_id {
        let binding =
            wiki_service::ensure_project_binding(conn, vault_id, &db_instance_id, root_id).await?;
        Some(serde_json::to_string(&vec![binding.id]).unwrap_or_else(|_| "[]".into()))
    } else {
        None
    };
    wiki_service::insert_local_session_source(
        conn,
        wiki_service::NewLocalSessionSource {
            id: source_id.clone(),
            vault_id: vault_id.to_string(),
            conversation_id: conv.id,
            folder_id: Some(conv.folder_id),
            root_folder_id,
            agent_type: Some(conv.agent_type.clone()),
            model: conv.model.clone(),
            captured_at: now,
            occurred_at: Some(conv.updated_at),
            source_title: conv.title.clone(),
            raw_path: String::new(),
            raw_hash: hash,
            project_ids,
        },
    )
    .await?;
    Ok(source_id)
}

/// Fixture-friendly markdown writer. Host tests feed a `ConversationDetail`.
pub fn session_export_markdown(detail: &ConversationDetail) -> String {
    crate::wiki::session_import::session_detail_to_markdown(detail)
}

pub fn render_local_session_raw(
    source_id: &str,
    conversation_id: i32,
    title: &str,
    agent_type: &str,
    folder_path: Option<&str>,
    body: &str,
) -> (String, String) {
    let hash = raw::content_hash(body);
    let mut yaml = String::from("---\n");
    yaml.push_str("title: ");
    yaml.push_str(&turn_summary::yaml_quote(title));
    yaml.push('\n');
    yaml.push_str("type: session-dump\n");
    yaml.push_str("tags:\n  - \"type/session-dump\"\n");
    yaml.push_str("source_kind: local-session\n");
    yaml.push_str("codeg_source_id: ");
    yaml.push_str(&turn_summary::yaml_quote(source_id));
    yaml.push('\n');
    yaml.push_str("codeg_conversation_id: ");
    yaml.push_str(&conversation_id.to_string());
    yaml.push('\n');
    yaml.push_str("agent: ");
    yaml.push_str(&turn_summary::yaml_quote(agent_type));
    yaml.push('\n');
    if let Some(fp) = folder_path {
        yaml.push_str("folder_path: ");
        yaml.push_str(&turn_summary::yaml_quote(fp));
        yaml.push('\n');
    }
    yaml.push_str("codeg_content_hash: ");
    yaml.push_str(&turn_summary::yaml_quote(&hash));
    yaml.push_str("\n---\n\n");
    yaml.push_str(body);
    if !body.ends_with('\n') {
        yaml.push('\n');
    }
    (yaml, hash)
}

fn export_conversation_transcript(
    conv: &crate::db::entities::conversation::Model,
) -> Result<String, DbError> {
    let Some(external_id) = conv
        .external_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err(DbError::Validation(
            "local-session has no transcript to export".into(),
        ));
    };
    let Some(agent) = AgentType::from_wire(&conv.agent_type) else {
        return Err(DbError::Validation(format!(
            "unknown agent type {}",
            conv.agent_type
        )));
    };
    let detail = crate::parsers::build_agent_parser(agent)
        .get_conversation(external_id)
        .map_err(|e| DbError::Validation(format!("export local session: {e}")))?;
    let md = session_export_markdown(&detail);
    if md.trim().is_empty() {
        return Err(DbError::Validation(
            "local-session export produced no text".into(),
        ));
    }
    Ok(md)
}

pub async fn run_session_rollup_job(
    conn: &DatabaseConnection,
    job: &wiki_job::Model,
    llm: &dyn WikiLlm,
    vault: &Path,
    state_root: &Path,
) -> Result<Value, WorkerError> {
    vault::initialize_vault(vault).map_err(|e| WorkerError::Failed(e.to_string()))?;
    vault::initialize_state_root(state_root).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let conversation_id = conversation_id_from_job(job)
        .ok_or_else(|| WikiLlmError::InvalidOutput("session job missing conversation_id".into()))?;
    let rel = page_rel(conversation_id);
    let sources =
        wiki_service::list_sources_for_conversation(conn, &job.vault_id, conversation_id).await?;
    if sources.is_empty() {
        return Err(WikiLlmError::SourceMissing(format!("conversation {conversation_id}")).into());
    }
    let staging = state_root
        .join("staging")
        .join(&job.id)
        .join(job.attempt.to_string());
    fs::create_dir_all(&staging).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let staging = fs::canonicalize(&staging).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let mut paths = Vec::new();
    let mut turn_rels = Vec::new();
    let mut source_references = Vec::new();
    let mut source_warnings = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let turn = turn_summary::page_rel(&source.id);
        let (identity, readable) =
            if source.source_kind == "acp-turn" && read_vault_material(vault, &turn)?.is_some() {
                turn_rels.push(turn.clone());
                (turn.clone(), turn)
            } else {
                let (material, warning) = source_material(conn, source, vault, state_root).await?;
                source_warnings.extend(warning);
                let path = staging.join(format!("source-{index:04}.md"));
                fs::write(&path, material).map_err(|e| WorkerError::Failed(e.to_string()))?;
                (
                    format!("source:{}", source.id),
                    path.to_string_lossy().into_owned(),
                )
            };
        paths.push((identity, vec![source.id.clone()]));
        source_references.push(crate::wiki::llm::SourceReference {
            rel: readable,
            source_ids: vec![source.id.clone()],
        });
    }
    let required = turn_summary::source_inputs(&paths)?;
    let existing = turn_summary::read_existing(vault, &rel)?;
    let input = json!({
        "schema": crate::wiki::llm::SESSION_ROLLUP_CONTRACT_VERSION,
        "job_id": job.id, "attempt": job.attempt,
        "conversation_id": conversation_id, "rel": rel,
        "turn_rels": turn_rels, "source_references": source_references,
        "source_warnings": source_warnings,
        "vault_abs": vault, "staging_abs": staging,
        "max_turns": WIKI_SESSION_ROLLUP_MAX_TURNS,
    });
    let run = llm.complete_json(KIND, input).await?;
    let mut parsed = validate_session_rollup(&run.output, conversation_id)?;
    parsed.warnings.extend(source_warnings);
    if parsed.nothing_to_summarize {
        crate::wiki::worker::ensure_commit_allowed(conn, &job.id, llm).await?;
        return turn_summary::save_result(
            conn,
            job,
            &JobOutputManifest::no_content(
                &required,
                parsed
                    .reason_code
                    .as_deref()
                    .unwrap_or("no_durable_content"),
                parsed.warnings,
            ),
        )
        .await;
    }
    let note_id = existing
        .as_deref()
        .and_then(|t| yaml_string(t, "codeg_note_id"))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let binding = sources
        .iter()
        .find_map(|s| first_project_id(&s.project_ids));
    let after = turn_summary::wrap_memory_page(MemoryPageFields {
        title: &parsed.title,
        page_type: PAGE_TYPE,
        note_id: &note_id,
        source_id: None,
        conversation_id: Some(conversation_id),
        occurred_at: sources.iter().filter_map(|s| s.occurred_at).max(),
        project_binding_id: binding.as_deref(),
        turn_rels: &turn_rels,
        body: &parsed.body,
        source_references: &source_references,
    });
    check_leaf_body(PAGE_TYPE, &after)?;
    crate::wiki::worker::ensure_commit_allowed(conn, &job.id, llm).await?;
    let proposal = StagedProposal {
        rel: rel.clone(),
        page_type: PAGE_TYPE.into(),
        before_hash: existing
            .as_deref()
            .map(raw::content_hash)
            .unwrap_or_default(),
        after,
        op: if existing.is_some() {
            "update".into()
        } else {
            "create".into()
        },
    };
    commit::commit_proposals(vault, state_root, &job.id, &[proposal])
        .map_err(CompileError::from)?;
    let actual =
        fs::read_to_string(vault.join(&rel)).map_err(|e| WorkerError::Failed(e.to_string()))?;
    let result = JobOutputManifest::generated(
        vec![WikiOutput {
            note_id,
            path: rel,
            title: parsed.title,
            page_type: PAGE_TYPE.into(),
            content_hash: raw::content_hash(&actual),
        }],
        &required,
        parsed.warnings,
    );
    let out = turn_summary::save_result(conn, job, &result).await?;
    turn_summary::register_memory_contributions(conn, job, &result).await?;
    commit::mark_finalized(state_root, &job.id).map_err(CompileError::from)?;
    Ok(out)
}

/// 来源身份保留在结果中；正文只物化到当前 attempt 的 staging，供文件工具读取。
async fn source_material(
    conn: &DatabaseConnection,
    source: &wiki_source::Model,
    vault: &Path,
    state_root: &Path,
) -> Result<(String, Option<String>), WorkerError> {
    if source.source_kind == "acp-turn" {
        let jobs = wiki_job::Entity::find()
            .filter(wiki_job::Column::VaultId.eq(&source.vault_id))
            .filter(wiki_job::Column::SourceId.eq(&source.id))
            .filter(wiki_job::Column::Kind.eq(turn_summary::KIND))
            .order_by_desc(wiki_job::Column::CreatedAt)
            .all(conn)
            .await
            .map_err(DbError::from)?;
        for job in jobs {
            let Some(manifest) = job.input_manifest else {
                continue;
            };
            let manifest: Value = serde_json::from_str(&manifest)
                .map_err(|e| WikiLlmError::SourceReadFailed(format!("{}: {e}", source.id)))?;
            if let Some(material) = manifest
                .get("material_markdown")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
            {
                return Ok((crate::wiki::redact::redact_text(material).0, None));
            }
        }
    }
    let snapshot_fallback = if source.source_kind == "local-session" {
        match local_session_material(conn, source, state_root).await {
            Ok(material) => return Ok((material, None)),
            Err(WorkerError::Llm(WikiLlmError::SourceMissing(_))) => true,
            Err(error) => return Err(error),
        }
    } else {
        false
    };
    if let Some(rel) = source
        .raw_path
        .as_deref()
        .map(str::trim)
        .filter(|rel| !rel.is_empty())
    {
        if let Some(material) = read_vault_material(vault, rel)? {
            let warning = snapshot_fallback.then(|| {
                format!(
                    "Current session is unavailable; source {} uses saved snapshot {rel} and may omit later turns.",
                    source.id
                )
            });
            return Ok((crate::wiki::redact::redact_text(&material).0, warning));
        }
    }
    Err(WikiLlmError::SourceMissing(source.id.clone()).into())
}

/// 使用真实路径限制宿主读取，避免把越界软链接的内容复制进可读 staging。
fn read_vault_material(vault: &Path, rel: &str) -> Result<Option<String>, WorkerError> {
    let path = crate::wiki::paths::join_vault_relative(vault, rel)
        .map_err(WikiLlmError::SourceReadFailed)?;
    let canonical = match fs::canonicalize(&path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WikiLlmError::SourceReadFailed(format!("{rel}: {error}")).into()),
    };
    let root =
        fs::canonicalize(vault).map_err(|e| WikiLlmError::SourceReadFailed(e.to_string()))?;
    if !canonical.starts_with(&root) {
        return Err(
            WikiLlmError::SourceReadFailed(format!("source path escapes vault: {rel}")).into(),
        );
    }
    fs::read_to_string(canonical)
        .map(Some)
        .map_err(|e| WikiLlmError::SourceReadFailed(format!("{rel}: {e}")).into())
}

async fn local_session_material(
    conn: &DatabaseConnection,
    source: &wiki_source::Model,
    state_root: &Path,
) -> Result<String, WorkerError> {
    // continuation 后 locator 仍可能保留旧 session ID，当前会话行才指向最新完整链。
    let current = if let Some(id) = source.conversation_id {
        conversation::Entity::find_by_id(id)
            .one(conn)
            .await
            .map_err(DbError::from)?
            .and_then(|conv| {
                conv.external_id
                    .filter(|id| !id.trim().is_empty())
                    .map(|session| (conv.agent_type, session))
            })
    } else {
        None
    };
    let (agent_type, session_id) = match current {
        Some(identity) => identity,
        None => {
            let locator =
                crate::wiki::locator::read_locator(state_root, &source.id).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        WikiLlmError::SourceMissing(source.id.clone())
                    } else {
                        WikiLlmError::SourceReadFailed(format!("{}: {error}", source.id))
                    }
                })?;
            let identity = locator
                .agent_type
                .zip(locator.session_id)
                .filter(|(agent, session)| !agent.trim().is_empty() && !session.trim().is_empty());
            identity.ok_or_else(|| WikiLlmError::SourceMissing(source.id.clone()))?
        }
    };
    let agent = AgentType::from_wire(&agent_type).ok_or_else(|| {
        WikiLlmError::SourceReadFailed(format!("unknown agent type {agent_type}"))
    })?;
    let source_id = source.id.clone();
    let markdown = tokio::task::spawn_blocking(move || {
        let detail = crate::parsers::build_agent_parser(agent)
            .get_conversation(&session_id)
            .map_err(|error| match error {
                crate::parsers::ParseError::ConversationNotFound(_) => {
                    WikiLlmError::SourceMissing(source_id.clone())
                }
                crate::parsers::ParseError::Io(ref io)
                    if io.kind() == std::io::ErrorKind::NotFound =>
                {
                    WikiLlmError::SourceMissing(source_id.clone())
                }
                _ => WikiLlmError::SourceReadFailed(format!("{source_id}: {error}")),
            })?;
        if detail.turns.is_empty() {
            return Err(WikiLlmError::SourceMissing(source_id));
        }
        Ok(crate::wiki::redact::redact_text(&session_export_markdown(&detail)).0)
    })
    .await
    .map_err(|e| WorkerError::Failed(e.to_string()))??;
    Ok(markdown)
}

fn validate_session_rollup(
    v: &Value,
    conversation_id: i32,
) -> Result<turn_summary::ParsedTurn, WorkerError> {
    if v.get("conversation_id").and_then(Value::as_i64) != Some(i64::from(conversation_id)) {
        return Err(WikiLlmError::InvalidOutput(
            "conversation_id must match the supplied conversation".into(),
        )
        .into());
    }
    turn_summary::validate_memory_output(v, crate::wiki::llm::SESSION_ROLLUP_CONTRACT_VERSION)
}

fn first_project_id(raw: &Option<String>) -> Option<String> {
    let s = raw.as_deref()?.trim();
    if s.is_empty() {
        return None;
    }
    let ids: Vec<String> = serde_json::from_str(s).ok()?;
    ids.into_iter().find(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::{conversation, wiki_source};
    use crate::db::service::conversation_service;
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::{ContentBlock, ConversationSummary, MessageTurn, TurnRole};
    use crate::wiki::settings::WikiSettings;
    use crate::wiki::vault;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

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
    fn export_writer_keeps_user_and_assistant() {
        let detail = ConversationDetail {
            summary: ConversationSummary {
                id: "s1".into(),
                agent_type: AgentType::CodegAgent,
                folder_path: Some("/tmp/proj".into()),
                folder_name: Some("proj".into()),
                title: Some("Local work".into()),
                started_at: Utc::now(),
                ended_at: None,
                message_count: 2,
                model: None,
                git_branch: None,
                parent_id: None,
                parent_tool_use_id: None,
                delegation_call_id: None,
            },
            turns: vec![
                turn(TurnRole::User, "ship pagination"),
                turn(TurnRole::Assistant, "updated the list handler"),
            ],
            session_stats: None,
            transcript_watermark: None,
        };
        let md = session_export_markdown(&detail);
        assert!(md.contains("# Local work"));
        assert!(md.contains("ship pagination"));
        assert!(md.contains("updated the list handler"));
        let (raw_doc, hash) = render_local_session_raw(
            "sid",
            7,
            "Local work",
            "codeg_agent",
            Some("/tmp/proj"),
            &md,
        );
        assert!(!hash.is_empty());
        assert!(raw_doc.contains("source_kind: local-session"));
        assert!(raw_doc.contains("codeg_conversation_id: 7"));
        assert!(raw_doc.contains("ship pagination"));
    }

    #[test]
    fn page_rel_is_stable_per_conversation() {
        assert_eq!(page_rel(42), "work/sessions/c42.md");
        assert_eq!(dedupe_key(42), "session_rollup:42");
    }

    async fn enable_wiki(conn: &DatabaseConnection, vault: &Path) {
        let s = WikiSettings {
            enabled: true,
            vault_path: Some(vault.to_string_lossy().to_string()),
            ..WikiSettings::default()
        };
        settings::save_settings(conn, &s).await.unwrap();
    }

    #[tokio::test]
    async fn completed_enqueues_session_rollup_pending_review_does_not() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-session").await;
        let cid = seed_conversation(&db, folder, AgentType::ClaudeCode).await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        conversation_service::update_status(&db.conn, cid, ConversationStatus::PendingReview)
            .await
            .unwrap();
        let jobs = wiki_service::list_jobs(&db.conn, 20, 0, None)
            .await
            .unwrap();
        assert!(
            jobs.iter().all(|j| j.kind != KIND),
            "PendingReview must not enqueue session_rollup"
        );

        conversation_service::update_status(&db.conn, cid, ConversationStatus::Completed)
            .await
            .unwrap();
        let jobs = wiki_service::list_jobs(&db.conn, 20, 0, None)
            .await
            .unwrap();
        // No acp-turn and no exportable transcript → enqueue inner may fail.
        // Force a local-session skip by inserting a source then completing again.
        let _ = jobs;
        let vault_row = wiki_service::ensure_active_vault(&db.conn, &dir.path().to_string_lossy())
            .await
            .unwrap();
        enqueue_session_job(&db.conn, &vault_row.id, cid)
            .await
            .unwrap();
        let jobs = wiki_service::list_jobs(&db.conn, 20, 0, None)
            .await
            .unwrap();
        assert!(jobs.iter().any(|j| j.kind == KIND && j.status == "queued"));
        let again = enqueue_session_job(&db.conn, &vault_row.id, cid)
            .await
            .unwrap();
        let queued: Vec<_> = jobs
            .iter()
            .filter(|j| j.kind == KIND && j.status == "queued")
            .collect();
        assert_eq!(queued.len(), 1);
        assert_eq!(again.id, queued[0].id);
    }

    #[tokio::test]
    async fn pending_review_status_write_does_not_call_enqueue_path() {
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/wiki-pr").await;
        let cid = seed_conversation(&db, folder, AgentType::ClaudeCode).await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        conversation_service::update_status_if(
            &db.conn,
            cid,
            ConversationStatus::InProgress,
            ConversationStatus::PendingReview,
        )
        .await
        .unwrap();
        let jobs = wiki_service::list_jobs(&db.conn, 20, 0, None)
            .await
            .unwrap();
        assert!(jobs.iter().all(|j| j.kind != KIND));
        let row = conversation::Entity::find_by_id(cid)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, ConversationStatus::PendingReview);
    }

    #[tokio::test]
    async fn wrap_session_page_only_under_work_sessions() {
        let page = turn_summary::wrap_memory_page(MemoryPageFields {
            title: "Shipped cursor pagination",
            page_type: PAGE_TYPE,
            note_id: "n1",
            source_id: None,
            conversation_id: Some(12),
            occurred_at: None,
            project_binding_id: None,
            turn_rels: &[String::from("work/turns/src.md")],
            body: "Implemented list cursors.",
            source_references: &[],
        });
        assert!(page.contains("type: session-summary"));
        assert!(page.contains("codeg_conversation_id: 12"));
        assert!(page.contains("[[work/turns/src]]"));
        assert!(!page.contains("type: turn-summary"));
    }

    #[test]
    fn vault_init_has_session_dir() {
        let dir = tempdir().unwrap();
        vault::initialize_vault(dir.path()).unwrap();
        assert!(dir.path().join("work/sessions").is_dir());
        assert!(dir.path().join("work/turns").is_dir());
    }

    struct MaterialFixture {
        dir: tempfile::TempDir,
        db: crate::db::AppDatabase,
        vault: PathBuf,
        state: PathBuf,
        source: wiki_source::Model,
        turn_job: wiki_job::Model,
        session_job: wiki_job::Model,
        conversation_id: i32,
    }

    async fn material_fixture() -> MaterialFixture {
        let dir = tempdir().unwrap();
        let vault = dir.path().join("vault");
        let state = dir.path().join("state");
        vault::initialize_vault(&vault).unwrap();
        let db = fresh_in_memory_db().await;
        let folder = seed_folder(&db, "/tmp/materials").await;
        let conversation_id = seed_conversation(&db, folder, AgentType::CodegAgent).await;
        let row = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
            .await
            .unwrap();
        let inserted = wiki_service::insert_acp_source_and_turn_job(
            &db.conn,
            wiki_service::NewAcpSource {
                vault_id: row.id.clone(),
                run_id: "run-1".into(),
                conversation_id: Some(conversation_id),
                folder_id: Some(folder),
                root_folder_id: Some(folder),
                agent_type: Some("codeg_agent".into()),
                model: None,
                mode: None,
                captured_at: Utc::now(),
                occurred_at: None,
                truncated: false,
                redacted: false,
                source_title: None,
            },
        )
        .await
        .unwrap();
        let session_job = enqueue_session_job(&db.conn, &row.id, conversation_id)
            .await
            .unwrap();
        MaterialFixture {
            dir,
            db,
            vault,
            state,
            source: inserted.source,
            turn_job: inserted.job.unwrap(),
            session_job,
            conversation_id,
        }
    }

    #[derive(Default)]
    struct InspectMaterialLlm(Mutex<Option<Value>>);

    #[async_trait::async_trait]
    impl WikiLlm for InspectMaterialLlm {
        async fn complete_json(
            &self,
            stage: &str,
            input: Value,
        ) -> Result<crate::wiki::llm::WikiLlmRun, WikiLlmError> {
            *self.0.lock().unwrap() = Some(input.clone());
            crate::wiki::llm::MockWikiLlm::default()
                .complete_json(stage, input)
                .await
        }
    }

    impl InspectMaterialLlm {
        fn material(&self, vault: &Path) -> (PathBuf, String) {
            let input = self.0.lock().unwrap();
            let rel = input.as_ref().unwrap()["source_references"][0]["rel"]
                .as_str()
                .unwrap();
            let path = vault.join(rel);
            let text = fs::read_to_string(&path).expect("model source must be a readable file");
            (path, text)
        }
    }

    #[tokio::test]
    async fn rollup_materializes_saved_turn_without_persisting_staging_identity() {
        let f = material_fixture().await;
        wiki_service::set_job_input_manifest(&f.db.conn, &f.turn_job.id,
            &json!({"material_markdown":"User requested pagination. Assistant fixed cursor handling."}).to_string())
            .await.unwrap();
        let llm = InspectMaterialLlm::default();
        let out = run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state)
            .await
            .unwrap();
        let (path, text) = llm.material(&f.vault);
        assert!(path.starts_with(fs::canonicalize(f.state.join("staging")).unwrap()));
        assert!(text.contains("fixed cursor handling"));
        assert_eq!(
            out["processed_inputs"][0]["rel"],
            format!("source:{}", f.source.id)
        );
        assert_eq!(
            fs::read_dir(f.vault.join("raw/sessions")).unwrap().count(),
            0
        );
    }

    #[tokio::test]
    async fn rollup_rejects_missing_material_before_calling_model() {
        let f = material_fixture().await;
        let llm = InspectMaterialLlm::default();
        let error = run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state)
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), "source_missing");
        assert!(llm.0.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn rollup_reads_existing_legacy_material() {
        let f = material_fixture().await;
        fs::create_dir_all(f.vault.join("raw/sessions")).unwrap();
        fs::write(
            f.vault.join("raw/sessions/legacy.md"),
            "Legacy user request and completed implementation.",
        )
        .unwrap();
        let mut source: wiki_source::ActiveModel = f.source.clone().into();
        source.raw_path = Set(Some("raw/sessions/legacy.md".into()));
        source.update(&f.db.conn).await.unwrap();
        let llm = InspectMaterialLlm::default();
        run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state)
            .await
            .unwrap();
        let (_, text) = llm.material(&f.vault);
        assert!(text.contains("Legacy user request"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rollup_rejects_turn_note_symlink_outside_vault() {
        let f = material_fixture().await;
        let outside = f.dir.path().join("private.md");
        fs::write(&outside, "Private unrelated material.").unwrap();
        std::os::unix::fs::symlink(&outside, f.vault.join(turn_summary::page_rel(&f.source.id)))
            .unwrap();
        let llm = InspectMaterialLlm::default();
        let error = run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state)
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), "source_read_failed");
        assert!(llm.0.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn rollup_reports_saved_snapshot_fallback_to_model_and_result() {
        let f = material_fixture().await;
        fs::write(
            f.vault.join("raw/sessions/saved.md"),
            "Saved session material.",
        )
        .unwrap();
        let mut conv: conversation::ActiveModel =
            conversation::Entity::find_by_id(f.conversation_id)
                .one(&f.db.conn)
                .await
                .unwrap()
                .unwrap()
                .into();
        conv.external_id = Set(None);
        conv.update(&f.db.conn).await.unwrap();
        let mut source: wiki_source::ActiveModel = f.source.clone().into();
        source.source_kind = Set("local-session".into());
        source.raw_path = Set(Some("raw/sessions/saved.md".into()));
        source.update(&f.db.conn).await.unwrap();
        let llm = InspectMaterialLlm::default();
        let out = run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state)
            .await
            .unwrap();
        let warnings = out["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        let warning = warnings[0].as_str().unwrap();
        assert!(warning.contains(&f.source.id));
        assert!(warning.contains("raw/sessions/saved.md"));
        assert_eq!(
            llm.0.lock().unwrap().as_ref().unwrap()["source_warnings"],
            out["warnings"]
        );
        assert_eq!(llm.material(&f.vault).1, "Saved session material.");
    }

    #[tokio::test]
    async fn rollup_exports_current_local_session_and_redacts_material() {
        let f = material_fixture().await;
        let codeg_root = f.dir.path().join("codeg");
        let codeg_root_text = codeg_root.to_string_lossy().into_owned();
        temp_env::async_with_vars([("CODEG_HOME", Some(codeg_root_text.as_str()))], async {
            let sessions = crate::paths::codeg_agent_sessions_root();
            for (id, previous, body) in [("old", None, "first request"), ("new", Some("old"), "follow up API_KEY=sk-live-test-secret")] {
                let mut header = crate::acp_transcript::TranscriptHeader::new("codeg_agent", id, "/tmp/materials", 1_750_000_000_000);
                if let Some(previous) = previous { header = header.continuing(previous); }
                crate::acp_transcript::append_line_in(&sessions, "project", id, &serde_json::to_string(&header).unwrap());
                crate::acp_transcript::append_line_in(&sessions, "project", id,
                    &json!({"t":1_750_000_000_001_u64,"k":"prompt","p":[{"type":"text","text":body}]}).to_string());
            }
            let mut conv: conversation::ActiveModel = conversation::Entity::find_by_id(f.conversation_id).one(&f.db.conn).await.unwrap().unwrap().into();
            conv.external_id = Set(Some("new".into()));
            conv.update(&f.db.conn).await.unwrap();
            let mut source: wiki_source::ActiveModel = f.source.clone().into();
            source.source_kind = Set("local-session".into());
            fs::write(f.vault.join("raw/sessions/old.md"), "Legacy snapshot before continuation.").unwrap();
            source.raw_path = Set(Some("raw/sessions/old.md".into()));
            source.update(&f.db.conn).await.unwrap();
            crate::wiki::locator::write_locator(&f.state, &crate::wiki::locator::SourceLocator {
                source_id: f.source.id.clone(), kind: "local-session".into(),
                agent_type: Some("codeg_agent".into()), session_id: Some("old".into()),
                ..Default::default()
            }).unwrap();
            let llm = InspectMaterialLlm::default();
            run_session_rollup_job(&f.db.conn, &f.session_job, &llm, &f.vault, &f.state).await.unwrap();
            let (path, text) = llm.material(&f.vault);
            assert!(path.starts_with(fs::canonicalize(f.state.join("staging")).unwrap()));
            assert!(text.contains("first request"));
            assert!(text.contains("follow up"), "current session must supersede the old locator");
            assert!(!text.contains("sk-live-test-secret"));
            assert!(text.contains("[REDACTED]"));
        }).await;
    }
}
