//! 组合 Markdown 与数据库，提供 Wiki 笔记、资料、任务、搜索和概览查询。
//! 每次请求使用同一个 Wiki 目录和身份；筛选在分页前完成，文件读取放到阻塞任务执行。
//! 来源存在性和产物状态在读取时计算；不触发模型、不补历史输入冻结或逐行证明。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};

use super::catalog::{self, CatalogEntry};
use super::document::Document;
use super::{
    page, WikiNoteDetail, WikiNoteQuery, WikiNoteSummary, WikiOverview, WikiPage,
    WikiSourceDocument, WikiSourceReference,
};
use crate::db::entities::{wiki_contribution, wiki_job, wiki_source};
use crate::db::error::DbError;
use crate::db::service::wiki_service;
use crate::wiki::{paths, settings};

/// Capture configuration and database identity once per request. A settings
/// change may invalidate this response, but cannot mix two vaults inside it.
struct ReadContext {
    root: PathBuf,
    config: settings::WikiSettings,
    active: Option<crate::db::entities::wiki_vault::Model>,
}
impl ReadContext {
    async fn load(conn: &DatabaseConnection) -> Result<Self, DbError> {
        let _transition = crate::wiki::lifecycle::lock().await;
        let config = settings::load_settings(conn).await?;
        let root = paths::resolve_vault_path(config.vault_path.as_deref());
        let active = wiki_service::active_vault(conn).await?;
        Ok(Self {
            root,
            config,
            active,
        })
    }
}

async fn catalog(context: &ReadContext) -> Result<Vec<CatalogEntry>, DbError> {
    let root = context.root.clone();
    tokio::task::spawn_blocking(move || catalog::scan(&root))
        .await
        .map_err(|e| DbError::Validation(format!("reading catalog failed: {e}")))?
}

async fn file(context: &ReadContext, rel: &str) -> Result<String, DbError> {
    let root = context.root.clone();
    let rel = rel.to_string();
    tokio::task::spawn_blocking(move || catalog::read_file(&root, &rel))
        .await
        .map_err(|e| DbError::Validation(format!("reading document failed: {e}")))?
}

/// 高级文件浏览与正文阅读共用目录边界、大小限制和文件读取规则。
pub async fn read_vault_file(
    conn: &DatabaseConnection,
    path: String,
) -> Result<super::WikiVaultFile, DbError> {
    let context = ReadContext::load(conn).await?;
    let content = file(&context, &path).await?;
    Ok(super::WikiVaultFile { path, content })
}

/// 任务历史记录来自数据库，产物状态来自本次读取时的文件系统。
/// 数据库服务不负责探测文件，避免工作线程查询任务时隐含读取整篇笔记。
pub async fn read_job(
    conn: &DatabaseConnection,
    id: &str,
) -> Result<wiki_service::WikiJobInfo, DbError> {
    let mut info = wiki_service::get_job(conn, id).await?;
    let vault = crate::db::entities::wiki_vault::Entity::find_by_id(&info.vault_id)
        .one(conn)
        .await?;
    let Some(vault) = vault else {
        return Ok(info);
    };
    tokio::task::spawn_blocking(move || {
        if let Some(result) = &info.result {
            for output in &result.outputs {
                let availability = match catalog::read_file(
                    std::path::Path::new(&vault.canonical_path),
                    &output.path,
                ) {
                    Ok(text) if crate::wiki::raw::content_hash(&text) == output.content_hash => {
                        "available"
                    }
                    Ok(_) => "conflict",
                    Err(_) => "missing",
                };
                info.output_availability
                    .insert(output.path.clone(), availability.into());
            }
        }
        info
    })
    .await
    .map_err(|error| DbError::Validation(format!("reading job outputs failed: {error}")))
}

pub async fn list_notes(
    conn: &DatabaseConnection,
    query: WikiNoteQuery,
) -> Result<WikiPage<WikiNoteSummary>, DbError> {
    let context = ReadContext::load(conn).await?;
    let scope = query.scope.as_deref().unwrap_or("notes");
    if !matches!(scope, "notes" | "sources" | "all") {
        return Err(DbError::Validation("invalid Wiki search scope".into()));
    }
    let text = query.query.as_deref().unwrap_or("").trim().to_lowercase();
    let mut matches = Vec::new();
    if scope != "sources" {
        for entry in catalog(&context).await? {
            let mut note = entry.summary;
            if query.view.as_deref() == Some("work") && !note.path.starts_with("work/") {
                continue;
            }
            if query.view.as_deref() == Some("capabilities") && note.page_type != "capability" {
                continue;
            }
            if query
                .page_type
                .as_deref()
                .is_some_and(|t| t != "all" && t != note.page_type)
            {
                continue;
            }
            if query
                .project_id
                .as_deref()
                .is_some_and(|p| !note.project_ids.iter().any(|id| id == p))
            {
                continue;
            }
            let title_match = note.title.to_lowercase().contains(&text);
            if !text.is_empty() && !title_match && !entry.body.to_lowercase().contains(&text) {
                continue;
            }
            if !text.is_empty() {
                note.excerpt = Some(catalog::excerpt(&entry.body, &text));
            }
            matches.push((title_match, note));
        }
    }
    if scope != "notes" {
        let root = &context.root;
        for source in source_models(conn, &context).await? {
            let projects: Vec<String> = source
                .project_ids
                .as_deref()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_default();
            if query
                .project_id
                .as_ref()
                .is_some_and(|id| !projects.contains(id))
            {
                continue;
            }
            let title = source_title(&source);
            let title_match = title.to_lowercase().contains(&text);
            let rel = source.raw_path.as_deref().unwrap_or("");
            let body = if text.is_empty() {
                String::new()
            } else {
                file(&context, rel)
                    .await
                    .map(|raw| Document::parse(&raw).body)
                    .unwrap_or_default()
            };
            if !text.is_empty() && !title_match && !body.to_lowercase().contains(&text) {
                continue;
            }
            let mut note = catalog::summary(root, rel, &Document::parse(&body));
            note.note_id = format!("source:{}", source.id);
            note.title = title;
            note.page_type = "source".into();
            note.project_ids = projects;
            note.source_id = Some(source.id.clone());
            note.source_ids = vec![source.id];
            note.updated_at = Some(source.updated_at.to_rfc3339());
            note.excerpt = (!text.is_empty()).then(|| catalog::excerpt(&body, &text));
            matches.push((title_match, note));
        }
    }
    matches.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.updated_at.cmp(&a.1.updated_at))
            .then(a.1.note_id.cmp(&b.1.note_id))
    });
    Ok(page(
        matches.into_iter().map(|(_, note)| note).collect(),
        query.offset,
        query.limit,
    ))
}

pub async fn read_note(
    conn: &DatabaseConnection,
    path: Option<String>,
    note_id: Option<String>,
) -> Result<WikiNoteDetail, DbError> {
    let context = ReadContext::load(conn).await?;
    let rel = match (path, note_id) {
        (Some(path), None) => path,
        (None, Some(id)) => {
            let found: Vec<_> = catalog(&context)
                .await?
                .into_iter()
                .filter(|entry| entry.summary.note_id == id)
                .collect();
            if found.len() > 1 {
                return Err(DbError::Conflict(
                    "multiple notes have the same identity".into(),
                ));
            }
            found
                .into_iter()
                .next()
                .ok_or_else(|| DbError::NotFound("Wiki note".into()))?
                .summary
                .path
        }
        _ => {
            return Err(DbError::Validation(
                "provide exactly one of path or noteId".into(),
            ))
        }
    };
    let source = file(&context, &rel).await?;
    let document = Document::parse(&source);
    let root = &context.root;
    let mut note = catalog::summary(root, &rel, &document);
    let contributions = wiki_contribution::Entity::find()
        .filter(wiki_contribution::Column::NoteId.eq(&note.note_id))
        .all(conn)
        .await?;
    note.source_ids
        .extend(contributions.into_iter().map(|c| c.source_id));
    note.source_ids.sort();
    note.source_ids.dedup();
    let source_map: HashMap<_, _> = source_models(conn, &context)
        .await?
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();
    let mut references = Vec::new();
    // Structured evidence carries exact frozen-file locations. Legacy free text
    // isn't promoted to verified line evidence; valid source IDs still open raw.
    if let Some(items) = document
        .metadata
        .get("evidence")
        .and_then(|v| v.as_sequence())
    {
        for evidence in items {
            let input_rel = evidence
                .get("input_rel")
                .or_else(|| evidence.get("path"))
                .or_else(|| evidence.get("rel"))
                .and_then(|v| v.as_str());
            let Some(input_rel) = input_rel else { continue };
            let start_line = evidence
                .get("start_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let end_line = evidence
                .get("end_line")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let source = source_map
                .values()
                .find(|s| s.raw_path.as_deref() == Some(input_rel));
            let excerpt = if let (Some(start), Some(end)) = (start_line, end_line) {
                if start > 0 && end >= start {
                    file(&context, input_rel).await.ok().map(|text| {
                        text.lines()
                            .skip(start - 1)
                            .take((end - start + 1).min(40))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                } else {
                    None
                }
            } else {
                None
            };
            references.push(WikiSourceReference {
                availability: source_availability(&context.root, input_rel).into(),
                source_url: source.and_then(|source| source.source_url.clone()),
                source_id: source.map(|s| s.id.clone()),
                title: source
                    .map(source_title)
                    .unwrap_or_else(|| input_rel.to_string()),
                path: input_rel.to_string(),
                start_line,
                end_line,
                excerpt,
            });
        }
    }
    for id in &note.source_ids {
        if references
            .iter()
            .any(|r| r.source_id.as_deref() == Some(id))
        {
            continue;
        }
        if let Some(source) = source_map.get(id) {
            if let Some(path) = &source.raw_path {
                references.push(WikiSourceReference {
                    availability: source_availability(&context.root, path).into(),
                    source_url: source.source_url.clone(),
                    source_id: Some(id.clone()),
                    title: source_title(source),
                    path: path.clone(),
                    start_line: None,
                    end_line: None,
                    excerpt: None,
                });
            }
        }
    }
    // Plain source locators remain useful even without a database row or exact
    // line evidence. Existence is evaluated now, never stored as a frozen fact.
    let mut source_paths = document.strings("sources");
    source_paths.extend(document.strings("source_paths"));
    for locator in source_paths {
        let locator = locator.trim_start_matches("[[").trim_end_matches("]]");
        let locator = locator.split('|').next().unwrap_or(locator);
        let path = if locator.ends_with(".md") {
            locator.to_string()
        } else {
            format!("{locator}.md")
        };
        if locator.strip_prefix("sources/").is_some_and(|id| {
            note.source_ids
                .iter()
                .any(|source| source == id.trim_end_matches(".md"))
        }) {
            continue;
        }
        if !locator.contains('/') || references.iter().any(|r| r.path == path) {
            continue;
        }
        references.push(WikiSourceReference {
            availability: source_availability(&context.root, &path).into(),
            source_url: None,
            source_id: None,
            title: locator.into(),
            path,
            start_line: None,
            end_line: None,
            excerpt: None,
        });
    }
    Ok(WikiNoteDetail {
        note,
        body: document.body,
        source,
        format_warning: document.format_warning,
        sources: references,
        headings: document.headings,
    })
}

fn source_availability(root: &std::path::Path, rel: &str) -> &'static str {
    let present = paths::join_vault_relative(root, rel)
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .zip(root.canonicalize().ok())
        .is_some_and(|(path, root)| path.starts_with(root) && path.is_file());
    if present {
        "available"
    } else {
        "missing"
    }
}

pub async fn read_source_document(
    conn: &DatabaseConnection,
    id: &str,
) -> Result<WikiSourceDocument, DbError> {
    let context = ReadContext::load(conn).await?;
    let active = context
        .active
        .as_ref()
        .ok_or_else(|| DbError::NotFound("Wiki source".into()))?;
    let source = wiki_service::get_source_model(conn, id).await?;
    if source.vault_id != active.id {
        return Err(DbError::NotFound("Wiki source".into()));
    }
    let (raw, read_error) = match source.raw_path.as_deref() {
        Some(rel) => match file(&context, rel).await {
            Ok(raw) => (raw, None),
            Err(error) => (String::new(), Some(error.to_string())),
        },
        None => (
            String::new(),
            Some("No extracted text is available; review the source information.".into()),
        ),
    };
    let doc = Document::parse(&raw);
    let note_ids: HashSet<_> = wiki_service::list_contributions_for_source(conn, id)
        .await?
        .into_iter()
        .map(|c| c.note_id)
        .collect();
    let related_notes = catalog(&context)
        .await?
        .into_iter()
        .map(|entry| entry.summary)
        .filter(|n| note_ids.contains(&n.note_id) || n.source_ids.iter().any(|source| source == id))
        .collect();
    Ok(WikiSourceDocument {
        read_error,
        source: wiki_service::source_info(source),
        body: doc.body,
        raw,
        format_warning: doc.format_warning,
        related_notes,
    })
}

async fn source_models(
    conn: &DatabaseConnection,
    context: &ReadContext,
) -> Result<Vec<wiki_source::Model>, DbError> {
    let Some(active) = context.active.as_ref() else {
        return Ok(Vec::new());
    };
    Ok(wiki_source::Entity::find()
        .filter(wiki_source::Column::VaultId.eq(&active.id))
        .filter(wiki_source::Column::Eligibility.ne("withdrawn"))
        .order_by_desc(wiki_source::Column::CreatedAt)
        .order_by_asc(wiki_source::Column::Id)
        .all(conn)
        .await?)
}

fn source_title(source: &wiki_source::Model) -> String {
    source
        .source_title
        .as_ref()
        .or(source.original_filename.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            if source.source_kind == "acp-turn" {
                "会话材料".into()
            } else {
                "导入资料".into()
            }
        })
}

pub async fn list_sources(
    conn: &DatabaseConnection,
    query: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<WikiPage<wiki_service::WikiSourceInfo>, DbError> {
    let context = ReadContext::load(conn).await?;
    let text = query.unwrap_or_default().trim().to_lowercase();
    let mut items = Vec::new();
    for source in source_models(conn, &context).await? {
        let mut matches = source_title(&source).to_lowercase().contains(&text);
        if !matches && !text.is_empty() {
            if let Some(path) = &source.raw_path {
                matches = file(&context, path)
                    .await
                    .is_ok_and(|raw| Document::parse(&raw).body.to_lowercase().contains(&text));
            }
        }
        if matches {
            items.push(wiki_service::source_info(source));
        }
    }
    Ok(page(items, offset, limit))
}

pub async fn list_jobs(
    conn: &DatabaseConnection,
    status: Option<String>,
    offset: Option<u64>,
    limit: Option<u64>,
) -> Result<WikiPage<wiki_service::WikiJobInfo>, DbError> {
    let context = ReadContext::load(conn).await?;
    let Some(active) = context.active.as_ref() else {
        return Ok(page(Vec::new(), offset, limit));
    };
    let mut query = wiki_job::Entity::find().filter(wiki_job::Column::VaultId.eq(&active.id));
    match status.as_deref() {
        None | Some("all") => {}
        Some("active") => {
            query = query.filter(wiki_job::Column::Status.is_in(["queued", "running"]))
        }
        Some("attention") => query = query.filter(wiki_job::Column::Status.eq("failed")),
        Some("generated" | "no_content" | "no_new_input") => {
            query = query.filter(wiki_job::Column::Status.eq("succeeded"))
        }
        Some("completed") => {
            query = query.filter(wiki_job::Column::Status.is_in(["succeeded", "cancelled"]))
        }
        Some(status)
            if ["queued", "running", "failed", "succeeded", "cancelled"].contains(&status) =>
        {
            query = query.filter(wiki_job::Column::Status.eq(status))
        }
        _ => return Err(DbError::Validation("invalid Wiki status filter".into())),
    }
    let mut rows = query
        .order_by_desc(wiki_job::Column::CreatedAt)
        .order_by_asc(wiki_job::Column::Id)
        .all(conn)
        .await?;
    if let Some(outcome @ ("generated" | "no_content" | "no_new_input")) = status.as_deref() {
        rows.retain(|row| {
            row.output_manifest
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .and_then(|v| {
                    v.get("outcome")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                == Some(outcome)
        });
    }
    let ids = page(rows, offset, limit);
    let mut items = Vec::new();
    for row in ids.items {
        items.push(read_job(conn, &row.id).await?);
    }
    Ok(WikiPage {
        items,
        total: ids.total,
        limit: ids.limit,
        offset: ids.offset,
    })
}

pub async fn get_overview(conn: &DatabaseConnection) -> Result<WikiOverview, DbError> {
    let context = ReadContext::load(conn).await?;
    let config = &context.config;
    let entries = catalog(&context).await?;
    let mut active_job_count = 0;
    let mut failed_job_count = 0;
    let mut source_count = 0;
    let mut pending_memory_count = 0;
    if let Some(active) = context.active.as_ref() {
        let jobs = wiki_job::Entity::find()
            .filter(wiki_job::Column::VaultId.eq(&active.id))
            .all(conn)
            .await?;
        active_job_count = jobs
            .iter()
            .filter(|j| matches!(j.status.as_str(), "queued" | "running"))
            .count();
        // New logical-key uniqueness means attempts cannot inflate these counts.
        failed_job_count = jobs.iter().filter(|j| j.status == "failed").count();
        source_count = wiki_source::Entity::find()
            .filter(wiki_source::Column::VaultId.eq(&active.id))
            .filter(wiki_source::Column::Eligibility.ne("withdrawn"))
            .count(conn)
            .await?;
        let root = paths::resolve_vault_path(config.vault_path.as_deref());
        let pending = crate::wiki::compile::list_pending_inputs(conn, &active.id, &root)
            .await
            .map_err(|e| DbError::Validation(e.to_string()))?;
        let claimed: HashSet<String> = jobs
            .iter()
            .filter(|j| {
                matches!(j.status.as_str(), "queued" | "running") && j.kind == "wiki_synthesize"
            })
            .filter_map(|j| j.input_manifest.as_deref())
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .flat_map(|manifest| {
                manifest
                    .get("memory_notes")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|input| {
                input
                    .get("rel")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        pending_memory_count = pending
            .memory_notes
            .iter()
            .filter(|n| !claimed.contains(&n.rel))
            .count();
    }
    Ok(WikiOverview {
        enabled: config.enabled,
        note_count: entries.len(),
        source_count,
        active_job_count,
        pending_memory_count,
        failed_job_count,
        recent_notes: entries
            .into_iter()
            .take(8)
            .map(|entry| entry.summary)
            .collect(),
        next_compile_at: settings::next_compile_at(config).map(|at| at.to_rfc3339()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::fresh_in_memory_db;
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    async fn fixture() -> (tempfile::TempDir, crate::db::AppDatabase, String) {
        let dir = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        let db = fresh_in_memory_db().await;
        let config = settings::WikiSettings {
            vault_path: Some(dir.path().to_string_lossy().into()),
            ..Default::default()
        };
        settings::save_settings(&db.conn, &config).await.unwrap();
        let active = wiki_service::ensure_active_vault(&db.conn, &dir.path().to_string_lossy())
            .await
            .unwrap();
        (dir, db, active.id)
    }

    async fn seed_source(
        conn: &DatabaseConnection,
        vault_id: &str,
        id: &str,
        title: &str,
        path: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
            "INSERT INTO wiki_source(id,source_group_id,vault_id,source_kind,source_seq,eligibility,annotation_revision,truncated,redacted,created_at,updated_at,source_title,raw_path) VALUES (?,?,?,'pasted-text',1,'ready',0,0,0,?,?,?,?)",
            vec![id.into(),id.into(),vault_id.into(),now.clone().into(),now.into(),title.into(),path.into()],
        )).await.unwrap();
    }

    #[tokio::test]
    async fn source_reader_uses_raw_when_wrapper_has_no_content() {
        let (dir, db, vault_id) = fixture().await;
        let rel = "raw/imports/source.md";
        std::fs::write(
            dir.path().join(rel),
            "---\ntitle: 原始资料\n---\n# 阅读正文\n完整的原始文本与关键结论。",
        )
        .unwrap();
        std::fs::write(dir.path().join("sources/source.md"), "# 来源\n暂无贡献。").unwrap();
        seed_source(&db.conn, &vault_id, "source", "原始资料", rel).await;
        let doc = read_source_document(&db.conn, "source").await.unwrap();
        assert!(doc.body.contains("完整的原始文本"));
        assert!(!doc.body.contains("暂无贡献"));
        assert!(!doc.body.contains("title:"));
        assert!(doc.related_notes.is_empty());
    }

    #[tokio::test]
    async fn chinese_search_filters_before_pagination_and_counts_only_content() {
        let (dir, db, vault_id) = fixture().await;
        for index in 0..61 {
            let rel = format!("raw/imports/{index}.md");
            std::fs::write(
                dir.path().join(&rel),
                format!("# 资料 {index}\n原始文本。目标条目{index}。"),
            )
            .unwrap();
            seed_source(
                &db.conn,
                &vault_id,
                &format!("s{index}"),
                &format!("资料 {index}"),
                &rel,
            )
            .await;
            std::fs::write(dir.path().join(format!("work/records/{index}.md")),format!("---\ncodeg_note_id: note-{index}\ntitle: 工作 {index}\ntype: work-record\n---\n# 工作\n已验证目标条目{index}，下一步检查分页边界。")).unwrap();
        }
        let second = list_sources(&db.conn, None, Some(30), Some(30))
            .await
            .unwrap();
        assert_eq!(second.total, 61);
        assert_eq!(second.items.len(), 30);
        let found = list_sources(&db.conn, Some("目标条目60".into()), None, None)
            .await
            .unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.items[0].id, "s60");
        let notes = list_notes(
            &db.conn,
            WikiNoteQuery {
                query: Some("目标条目60".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(notes.total, 1);
        assert_eq!(notes.items[0].note_id, "note-60");
        assert!(notes.items[0]
            .excerpt
            .as_ref()
            .unwrap()
            .contains("目标条目60"));
        let overview = get_overview(&db.conn).await.unwrap();
        assert_eq!(overview.note_count, 61);
        assert_eq!(overview.source_count, 61);
        assert_eq!(
            overview.pending_memory_count, 0,
            "archive-only sources aren't synthesis inputs"
        );
    }

    #[tokio::test]
    async fn rereading_same_path_observes_external_changes_and_deletion() {
        let (dir, db, _) = fixture().await;
        let rel = "work/records/note.md";
        std::fs::write(dir.path().join(rel), "# 正文\n原来的内容。").unwrap();
        let before = read_note(&db.conn, Some(rel.into()), None).await.unwrap();
        std::fs::write(dir.path().join(rel), "# 正文\n外部编辑器修改后的内容。").unwrap();
        let after = read_note(&db.conn, Some(rel.into()), None).await.unwrap();
        assert_ne!(before.body, after.body);
        std::fs::remove_file(dir.path().join(rel)).unwrap();
        assert!(read_note(&db.conn, Some(rel.into()), None).await.is_err());
    }

    #[tokio::test]
    async fn job_reader_adds_file_state_without_coupling_the_database_to_files() {
        let (dir, db, vault_id) = fixture().await;
        let rel = "work/records/output.md";
        let body = "# 工作\n保存的实际结果。";
        std::fs::write(dir.path().join(rel), body).unwrap();
        let job = wiki_service::insert_kind_job(
            &db.conn,
            &vault_id,
            "wiki_synthesize",
            "read-job-state",
            None,
            None,
            wiki_service::InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let manifest = serde_json::json!({
            "version":1, "outcome":"generated", "reason_code":null,
            "outputs":[{"note_id":"output", "path":rel, "title":"工作", "type":"work-record", "content_hash":crate::wiki::raw::content_hash(body)}],
            "processed_inputs":[], "remaining_inputs":[], "warnings":[]
        });
        wiki_service::set_job_output_manifest(&db.conn, &job.id, &manifest.to_string())
            .await
            .unwrap();
        assert!(wiki_service::get_job(&db.conn, &job.id)
            .await
            .unwrap()
            .output_availability
            .is_empty());
        assert_eq!(
            read_job(&db.conn, &job.id)
                .await
                .unwrap()
                .output_availability[rel],
            "available"
        );
        std::fs::write(dir.path().join(rel), "Human edit").unwrap();
        assert_eq!(
            read_job(&db.conn, &job.id)
                .await
                .unwrap()
                .output_availability[rel],
            "conflict"
        );
        std::fs::remove_file(dir.path().join(rel)).unwrap();
        assert_eq!(
            read_job(&db.conn, &job.id)
                .await
                .unwrap()
                .output_availability[rel],
            "missing"
        );
    }

    #[tokio::test]
    async fn source_existence_is_checked_at_read_time_without_invalidating_the_note() {
        let (dir, db, _) = fixture().await;
        let rel = "work/records/note.md";
        std::fs::write(
            dir.path().join(rel),
            "---\nsource_paths: [raw/imports/original.md]\n---\n# 笔记\n整理后的内容独立保存。\n",
        )
        .unwrap();
        let original = dir.path().join("raw/imports/original.md");
        std::fs::write(&original, "source text").unwrap();
        let first = read_note(&db.conn, Some(rel.into()), None).await.unwrap();
        assert_eq!(first.sources[0].availability, "available");
        std::fs::remove_file(original).unwrap();
        let second = read_note(&db.conn, Some(rel.into()), None).await.unwrap();
        assert_eq!(second.sources[0].availability, "missing");
        assert_eq!(first.body, second.body);
        assert_eq!(second.sources[0].path, "raw/imports/original.md");
    }
    #[tokio::test]
    async fn request_context_keeps_one_vault_when_selection_changes() {
        let (first, db, first_id) = fixture().await;
        std::fs::write(first.path().join("raw/imports/one.md"), "first vault body").unwrap();
        seed_source(
            &db.conn,
            &first_id,
            "first-source",
            "First",
            "raw/imports/one.md",
        )
        .await;
        let snapshot = ReadContext::load(&db.conn).await.unwrap();
        let second = tempfile::tempdir().unwrap();
        crate::wiki::vault::initialize_vault(second.path()).unwrap();
        std::fs::write(
            second.path().join("raw/imports/one.md"),
            "second vault body",
        )
        .unwrap();
        settings::save_settings(
            &db.conn,
            &settings::WikiSettings {
                vault_path: Some(second.path().to_string_lossy().into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        wiki_service::ensure_active_vault(&db.conn, &second.path().to_string_lossy())
            .await
            .unwrap();
        assert_eq!(
            file(&snapshot, "raw/imports/one.md").await.unwrap(),
            "first vault body"
        );
        assert_eq!(
            source_models(&db.conn, &snapshot).await.unwrap()[0].id,
            "first-source"
        );
        assert_eq!(
            list_sources(&db.conn, None, None, None)
                .await
                .unwrap()
                .total,
            0
        );
    }

    #[tokio::test]
    async fn failed_source_still_has_metadata_and_an_explicit_read_error() {
        let (_dir, db, vault_id) = fixture().await;
        seed_source(
            &db.conn,
            &vault_id,
            "no-text",
            "Encrypted PDF",
            "raw/imports/missing.md",
        )
        .await;
        let doc = read_source_document(&db.conn, "no-text").await.unwrap();
        assert_eq!(doc.source.id, "no-text");
        assert!(doc.read_error.is_some());
        assert!(doc.body.is_empty());
    }

    #[tokio::test]
    async fn source_search_respects_project_filter() {
        let (dir, db, vault_id) = fixture().await;
        std::fs::write(dir.path().join("raw/imports/one.md"), "project source").unwrap();
        seed_source(
            &db.conn,
            &vault_id,
            "project-source",
            "Shared title",
            "raw/imports/one.md",
        )
        .await;
        db.conn
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Sqlite,
                "UPDATE wiki_source SET project_ids=? WHERE id='project-source'",
                vec!["[\"project-a\"]".into()],
            ))
            .await
            .unwrap();
        let query = WikiNoteQuery {
            scope: Some("sources".into()),
            project_id: Some("project-a".into()),
            ..Default::default()
        };
        let page = list_notes(&db.conn, query).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].project_ids, ["project-a"]);
        assert_eq!(
            list_notes(
                &db.conn,
                WikiNoteQuery {
                    scope: Some("all".into()),
                    project_id: Some("project-b".into()),
                    ..Default::default()
                }
            )
            .await
            .unwrap()
            .total,
            0
        );
    }
}
