//! External document / pasted-text import. Host extract, hash, raw freeze.
//!
//! No model calls. Events (`wiki://job-changed`) are not emitted in this PR.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::app_error::AppCommandError;
use crate::db::error::DbError;
use crate::db::service::wiki_service::{
    self, AnnotationPatch, NewImportSource, WikiImportResult, WikiSourceInfo,
};
use crate::document_extract::{
    self, DocFormat, ExtractError, ExtractRequest, ExtractResult, ExtractionStatus, TextSegment,
    MAX_FILE_BYTES, MAX_NORMALIZED_CHARS,
};
use crate::wiki::paths::{resolve_state_root, resolve_vault_path};
use crate::wiki::raw::{self, RawImportMeta, RawWriteOutcome};
use crate::wiki::redact;
use crate::wiki::settings;
use crate::wiki::vault;

pub const MAX_IMPORT_FILES: usize = 20;
pub const DEFAULT_MATERIAL_ROLE: &str = "reference";

const MATERIAL_ROLES: &[&str] = &["reference", "own-work", "team-work", "unspecified"];

#[derive(Debug, Clone, Deserialize)]
pub struct ImportTextParams {
    pub request_id: String,
    pub text: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub material_role: Option<String>,
    #[serde(default)]
    pub personal_role: Option<String>,
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
    #[serde(default)]
    pub area_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFilePart {
    pub filename: String,
    #[serde(default)]
    pub mime: Option<String>,
    pub bytes_base64: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportFilesParams {
    pub request_id: String,
    pub files: Vec<ImportFilePart>,
    #[serde(default)]
    pub material_role: Option<String>,
    #[serde(default)]
    pub personal_role: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub batch_id: Option<String>,
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
    #[serde(default)]
    pub area_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAnnotationsParams {
    pub source_id: String,
    #[serde(default)]
    pub material_role: Option<String>,
    #[serde(default)]
    pub personal_role: Option<String>,
    #[serde(default)]
    pub project_ids: Option<Vec<String>>,
    #[serde(default)]
    pub area_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceIdParams {
    pub source_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkVersionParams {
    pub source_id: String,
    pub previous_source_id: String,
}

struct ImportPayload {
    request_id: String,
    source_kind: String,
    filename: String,
    mime: Option<String>,
    bytes: Vec<u8>,
    original_hash: String,
    title: Option<String>,
    source_url: Option<String>,
    author: Option<String>,
    material_role: String,
    personal_role: Option<String>,
    project_ids: Option<Vec<String>>,
    area_ids: Option<Vec<String>>,
    batch_id: Option<String>,
    skip_hash_dedup: bool,
    source_group_id: Option<String>,
    previous_source_id: Option<String>,
}

pub async fn import_text(
    conn: &DatabaseConnection,
    params: ImportTextParams,
) -> Result<WikiImportResult, AppCommandError> {
    let request_id = require_request_id(&params.request_id)?;
    if params.text.is_empty() {
        return Err(AppCommandError::invalid_input("pasted text is empty"));
    }
    let normalized = normalize_paste_text(&params.text);
    if normalized.chars().count() > MAX_NORMALIZED_CHARS {
        return Err(AppCommandError::invalid_input(format!(
            "pasted text exceeds {MAX_NORMALIZED_CHARS} characters"
        )));
    }
    let bytes = normalized.as_bytes().to_vec();
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppCommandError::invalid_input(format!(
            "pasted text exceeds {MAX_FILE_BYTES} bytes"
        )));
    }
    let original_hash = sha256_hex(&bytes);
    let filename = paste_filename(params.title.as_deref());
    ingest(
        conn,
        ImportPayload {
            request_id,
            source_kind: "pasted-text".into(),
            filename,
            mime: Some("text/markdown".into()),
            bytes,
            original_hash,
            title: empty_to_none(params.title),
            source_url: empty_to_none(params.source_url),
            author: empty_to_none(params.author),
            material_role: parse_material_role(params.material_role)?,
            personal_role: empty_to_none(params.personal_role),
            project_ids: params.project_ids,
            area_ids: params.area_ids,
            batch_id: None,
            skip_hash_dedup: false,
            source_group_id: None,
            previous_source_id: None,
        },
    )
    .await
}

pub async fn import_files(
    conn: &DatabaseConnection,
    params: ImportFilesParams,
) -> Result<WikiImportResult, AppCommandError> {
    let request_id = require_request_id(&params.request_id)?;
    if params.files.is_empty() {
        return Err(AppCommandError::invalid_input("no files provided"));
    }
    if params.files.len() > MAX_IMPORT_FILES {
        return Err(AppCommandError::invalid_input(format!(
            "at most {MAX_IMPORT_FILES} files per batch"
        )));
    }
    if params.files.len() != 1 {
        return Err(AppCommandError::invalid_input(
            "wiki_import_files accepts one file per request; the client should loop",
        ));
    }
    let file = &params.files[0];
    let bytes = decode_base64(&file.bytes_base64)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AppCommandError::invalid_input(format!(
            "file exceeds {MAX_FILE_BYTES} bytes (got {})",
            bytes.len()
        )));
    }
    if bytes.is_empty() {
        return Err(AppCommandError::invalid_input("file is empty"));
    }
    let filename = safe_filename(&file.filename);
    ingest(
        conn,
        ImportPayload {
            request_id,
            source_kind: "document".into(),
            filename,
            mime: empty_to_none(file.mime.clone()),
            original_hash: sha256_hex(&bytes),
            bytes,
            title: empty_to_none(params.title),
            source_url: empty_to_none(params.source_url),
            author: empty_to_none(params.author),
            material_role: parse_material_role(params.material_role)?,
            personal_role: empty_to_none(params.personal_role),
            project_ids: params.project_ids,
            area_ids: params.area_ids,
            batch_id: empty_to_none(params.batch_id),
            skip_hash_dedup: false,
            source_group_id: None,
            previous_source_id: None,
        },
    )
    .await
}

pub async fn accept_extraction(
    conn: &DatabaseConnection,
    source_id: String,
) -> Result<WikiSourceInfo, AppCommandError> {
    let row = wiki_service::get_source_model(conn, &source_id)
        .await
        .map_err(AppCommandError::from)?;
    if row.eligibility != "awaiting-acceptance" {
        return Err(AppCommandError::invalid_input(
            "source is not awaiting extraction acceptance",
        ));
    }
    let updated = wiki_service::set_source_eligibility(conn, &source_id, "ready")
        .await
        .map_err(AppCommandError::from)?;
    Ok(wiki_service::source_info(updated))
}

pub async fn update_source_annotations(
    conn: &DatabaseConnection,
    params: UpdateAnnotationsParams,
) -> Result<WikiSourceInfo, AppCommandError> {
    if params.material_role.is_none()
        && params.personal_role.is_none()
        && params.project_ids.is_none()
        && params.area_ids.is_none()
    {
        return Err(AppCommandError::invalid_input(
            "no annotation fields provided",
        ));
    }
    let material_role = match params.material_role {
        Some(role) => Some(parse_material_role(Some(role))?),
        None => None,
    };
    let updated = wiki_service::update_source_annotations(
        conn,
        &params.source_id,
        AnnotationPatch {
            material_role,
            personal_role: params.personal_role,
            project_ids: params.project_ids,
            area_ids: params.area_ids,
        },
    )
    .await
    .map_err(AppCommandError::from)?;
    Ok(wiki_service::source_info(updated))
}

pub async fn reextract(
    conn: &DatabaseConnection,
    source_id: String,
) -> Result<WikiImportResult, AppCommandError> {
    let row = wiki_service::get_source_model(conn, &source_id)
        .await
        .map_err(AppCommandError::from)?;
    if row.source_kind != "document" && row.source_kind != "pasted-text" {
        return Err(AppCommandError::invalid_input(
            "reextract is only supported for imported documents",
        ));
    }
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    if !settings.enabled {
        return Err(wiki_disabled());
    }
    let state_root = resolve_state_root(settings.vault_path.as_deref());
    let filename = row
        .original_filename
        .clone()
        .unwrap_or_else(|| "upload.bin".into());
    let original_path = originals_dir(&state_root, &row.id).join(&filename);
    if !original_path.is_file() {
        return Err(AppCommandError::not_found(format!(
            "original file missing for source {source_id}"
        )));
    }
    let bytes = fs::read(&original_path).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    let original_hash = row
        .original_hash
        .clone()
        .unwrap_or_else(|| sha256_hex(&bytes));
    ingest(
        conn,
        ImportPayload {
            request_id: format!("reextract:{}:{}", row.id, uuid::Uuid::new_v4()),
            source_kind: row.source_kind.clone(),
            filename,
            mime: None,
            bytes,
            original_hash,
            title: row.source_title.clone(),
            source_url: row.source_url.clone(),
            author: row.author.clone(),
            material_role: row
                .material_role
                .clone()
                .unwrap_or_else(|| DEFAULT_MATERIAL_ROLE.to_string()),
            personal_role: row.personal_role.clone(),
            project_ids: parse_json_list(&row.project_ids),
            area_ids: parse_json_list(&row.area_ids),
            batch_id: None,
            skip_hash_dedup: true,
            source_group_id: Some(row.source_group_id.clone()),
            previous_source_id: Some(row.id.clone()),
        },
    )
    .await
}

pub async fn link_source_version(
    conn: &DatabaseConnection,
    params: LinkVersionParams,
) -> Result<WikiSourceInfo, AppCommandError> {
    if params.source_id == params.previous_source_id {
        return Err(AppCommandError::invalid_input(
            "cannot link a source to itself",
        ));
    }
    let updated =
        wiki_service::link_source_version(conn, &params.source_id, &params.previous_source_id)
            .await
            .map_err(|e| match e {
                DbError::Validation(msg) => AppCommandError::invalid_input(msg),
                other => AppCommandError::from(other),
            })?;
    Ok(wiki_service::source_info(updated))
}

async fn ingest(
    conn: &DatabaseConnection,
    payload: ImportPayload,
) -> Result<WikiImportResult, AppCommandError> {
    let settings = settings::load_settings(conn)
        .await
        .map_err(AppCommandError::from)?;
    if !settings.enabled {
        return Err(wiki_disabled());
    }

    let vault_path = resolve_vault_path(settings.vault_path.as_deref());
    let state_root = resolve_state_root(settings.vault_path.as_deref());
    vault::initialize_vault(&vault_path).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    vault::initialize_state_root(&state_root)
        .map_err(|e| AppCommandError::io_error(e.to_string()))?;
    let canonical = vault_path.to_string_lossy().to_string();
    let vault_row = wiki_service::ensure_active_vault(conn, &canonical)
        .await
        .map_err(AppCommandError::from)?;
    let _ = settings::ensure_db_instance_id(conn).await;

    if let Some(existing) =
        wiki_service::find_source_by_request_id(conn, &vault_row.id, &payload.request_id)
            .await
            .map_err(AppCommandError::from)?
    {
        return Ok(WikiImportResult {
            source: wiki_service::source_info(existing),
            duplicate: false,
        });
    }
    if !payload.skip_hash_dedup {
        if let Some(existing) =
            wiki_service::find_source_by_original_hash(conn, &vault_row.id, &payload.original_hash)
                .await
                .map_err(AppCommandError::from)?
        {
            return Ok(WikiImportResult {
                source: wiki_service::source_info(existing),
                duplicate: true,
            });
        }
    }

    let extract_outcome = match document_extract::extract(ExtractRequest {
        bytes: &payload.bytes,
        filename: Some(&payload.filename),
        mime_hint: payload.mime.as_deref(),
    }) {
        Ok(result) => ExtractOutcome::from_result(result),
        Err(err) => match classify_extract_error(&err) {
            ExtractClass::Reject => return Err(AppCommandError::invalid_input(err.to_string())),
            ExtractClass::Fail => ExtractOutcome::failed(err),
        },
    };

    let source_id = uuid::Uuid::new_v4().to_string();
    store_original(&state_root, &source_id, &payload.filename, &payload.bytes)?;

    let (title, source_url, author, personal_role, redacted_meta) = redact_meta(
        payload.title.as_deref(),
        payload.source_url.as_deref(),
        payload.author.as_deref(),
        payload.personal_role.as_deref(),
    );

    let mut segments = extract_outcome.segments;
    let mut redacted = redacted_meta;
    for seg in &mut segments {
        let (text, changed) = redact::redact_text(&seg.text);
        seg.text = text;
        redacted |= changed;
    }
    let mut warnings = extract_outcome.warnings;
    for w in &mut warnings {
        let (text, changed) = redact::redact_text(w);
        *w = text;
        redacted |= changed;
    }

    let title = title.unwrap_or_else(|| {
        payload
            .filename
            .trim()
            .split('.')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Imported document")
            .to_string()
    });

    let mut raw_path = None;
    let mut raw_hash = None;
    let mut eligibility = extract_outcome.eligibility.clone();
    if extract_outcome.ok {
        let captured_at = Utc::now();
        let meta = RawImportMeta {
            source_id: &source_id,
            source_group_id: payload.source_group_id.as_deref().unwrap_or(&source_id),
            source_kind: &payload.source_kind,
            captured_at,
            title: &title,
            source_title: Some(title.as_str()),
            source_url: source_url.as_deref(),
            author: author.as_deref(),
            material_role: &payload.material_role,
            personal_role: personal_role.as_deref(),
            original_filename: Some(&payload.filename),
            format: &extract_outcome.format,
            extraction_status: extract_outcome
                .extraction_status
                .as_deref()
                .unwrap_or("complete"),
            extractor_version: extract_outcome.extractor_version.as_str(),
            redacted,
            truncated: false,
            page_count: extract_outcome.page_count,
            warnings: &warnings,
        };
        let (doc, hash) = raw::render_import_raw(&segments, &meta);
        match raw::write_import_raw(&vault_path, &source_id, &doc, &hash)
            .map_err(|e| AppCommandError::io_error(e.to_string()))?
        {
            RawWriteOutcome::Created { .. } | RawWriteOutcome::Identical { .. } => {}
            RawWriteOutcome::Conflict {
                existing_hash,
                new_hash,
                ..
            } => {
                return Err(AppCommandError::already_exists(format!(
                    "raw path exists with hash {existing_hash}, new hash {new_hash}"
                )));
            }
        }
        raw_path = Some(format!("raw/imports/{source_id}.md"));
        raw_hash = Some(hash);
    } else {
        eligibility = "failed".into();
    }

    let warnings_json = if warnings.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&warnings).unwrap_or_else(|_| "[]".into()))
    };
    let captured_at = Utc::now();
    let input_manifest = serde_json::json!({
        "request_id": payload.request_id,
        "filename": payload.filename,
        "original_hash": payload.original_hash,
        "batch_id": payload.batch_id,
        "kind": "import",
    })
    .to_string();

    let new = NewImportSource {
        id: Some(source_id.clone()),
        vault_id: vault_row.id,
        source_kind: payload.source_kind,
        request_id: payload.request_id,
        original_hash: payload.original_hash,
        original_filename: Some(payload.filename),
        format: Some(extract_outcome.format),
        source_title: Some(title),
        source_url,
        author,
        material_role: payload.material_role,
        personal_role,
        project_ids: payload
            .project_ids
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok()),
        area_ids: payload
            .area_ids
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok()),
        extractor_version: Some(extract_outcome.extractor_version),
        coverage_status: extract_outcome.extraction_status.clone(),
        eligibility: eligibility.clone(),
        warnings: warnings_json,
        page_count: extract_outcome.page_count.map(|n| n as i32),
        truncated: false,
        redacted,
        captured_at,
        source_group_id: payload.source_group_id,
        previous_source_id: payload.previous_source_id,
        skip_hash_dedup: payload.skip_hash_dedup,
        raw_path,
        raw_hash,
        job_status: if extract_outcome.ok {
            "succeeded".into()
        } else {
            "failed".into()
        },
        error_code: extract_outcome.error_code.clone(),
        error_message: extract_outcome.error_message.clone(),
        input_manifest: Some(input_manifest),
    };

    let inserted = wiki_service::insert_import_source_and_ingest_job(conn, new)
        .await
        .map_err(AppCommandError::from)?;

    if inserted.created {
        let log_line = format!(
            "{} ingest {} job={} source={} import={}",
            Utc::now().to_rfc3339(),
            if extract_outcome.ok {
                "succeeded"
            } else {
                "failed"
            },
            inserted.job.id,
            inserted.source.id,
            inserted.source.source_kind
        );
        raw::append_log_idempotent(&vault_path.join("log.md"), &inserted.job.id, &log_line)
            .map_err(|e| AppCommandError::io_error(e.to_string()))?;
    } else {
        let _ = fs::remove_dir_all(originals_dir(&state_root, &source_id));
        let leftover = raw::raw_import_path(&vault_path, &source_id);
        let _ = fs::remove_file(leftover);
    }

    let source = wiki_service::get_source(conn, &inserted.source.id)
        .await
        .map_err(AppCommandError::from)?;
    Ok(WikiImportResult {
        source,
        duplicate: !inserted.created,
    })
}

struct ExtractOutcome {
    ok: bool,
    segments: Vec<TextSegment>,
    warnings: Vec<String>,
    extractor_version: String,
    extraction_status: Option<String>,
    format: String,
    page_count: Option<u32>,
    eligibility: String,
    error_code: Option<String>,
    error_message: Option<String>,
}

impl ExtractOutcome {
    fn from_result(result: ExtractResult) -> Self {
        let format = format_label(result.format).to_string();
        if result
            .text_segments
            .iter()
            .all(|s| s.text.trim().is_empty())
        {
            return Self {
                ok: false,
                segments: Vec::new(),
                warnings: result.warnings,
                extractor_version: result.extractor_version.to_string(),
                extraction_status: None,
                format,
                page_count: result.page_count,
                eligibility: "failed".into(),
                error_code: Some("no_extractable_text".into()),
                error_message: Some("document produced no extractable text".into()),
            };
        }
        let extraction_status = match result.extraction_status {
            ExtractionStatus::Complete => "complete",
            ExtractionStatus::Partial => "partial",
        };
        let eligibility = match result.extraction_status {
            ExtractionStatus::Complete => "ready",
            ExtractionStatus::Partial => "awaiting-acceptance",
        };
        Self {
            ok: true,
            segments: result.text_segments,
            warnings: result.warnings,
            extractor_version: result.extractor_version.to_string(),
            extraction_status: Some(extraction_status.into()),
            format,
            page_count: result.page_count,
            eligibility: eligibility.into(),
            error_code: None,
            error_message: None,
        }
    }

    fn failed(err: ExtractError) -> Self {
        let (code, format) = match &err {
            ExtractError::EncryptedPdf => ("encrypted_pdf", "pdf"),
            ExtractError::NoExtractableText => ("no_extractable_text", "pdf"),
            ExtractError::InvalidEncoding(_) => ("invalid_encoding", "txt"),
            ExtractError::Parse(_) => ("extract_parse", "unknown"),
            _ => ("extract_failed", "unknown"),
        };
        Self {
            ok: false,
            segments: Vec::new(),
            warnings: Vec::new(),
            extractor_version: document_extract::EXTRACTOR_VERSION.to_string(),
            extraction_status: None,
            format: format.into(),
            page_count: None,
            eligibility: "failed".into(),
            error_code: Some(code.into()),
            error_message: Some(err.to_string()),
        }
    }
}

enum ExtractClass {
    Reject,
    Fail,
}

fn classify_extract_error(err: &ExtractError) -> ExtractClass {
    match err {
        ExtractError::FileTooLarge { .. }
        | ExtractError::TextTooLong { .. }
        | ExtractError::PdfTooManyPages { .. }
        | ExtractError::DocxLimits(_)
        | ExtractError::UnsupportedFormat { .. }
        | ExtractError::Timeout { .. } => ExtractClass::Reject,
        ExtractError::EncryptedPdf
        | ExtractError::NoExtractableText
        | ExtractError::InvalidEncoding(_)
        | ExtractError::Parse(_) => ExtractClass::Fail,
    }
}

fn format_label(fmt: DocFormat) -> &'static str {
    match fmt {
        DocFormat::Markdown => "markdown",
        DocFormat::PlainText => "txt",
        DocFormat::Pdf => "pdf",
        DocFormat::Docx => "docx",
    }
}

fn wiki_disabled() -> AppCommandError {
    AppCommandError::configuration_invalid(
        "Wiki is disabled; enable it in settings before importing.",
    )
}

fn require_request_id(id: &str) -> Result<String, AppCommandError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(AppCommandError::invalid_input("request_id is required"));
    }
    Ok(id.to_string())
}

fn parse_material_role(raw: Option<String>) -> Result<String, AppCommandError> {
    let Some(role) = raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_MATERIAL_ROLE.to_string());
    };
    if MATERIAL_ROLES.contains(&role.as_str()) {
        Ok(role)
    } else {
        Err(AppCommandError::invalid_input(format!(
            "material_role must be one of: {}",
            MATERIAL_ROLES.join(", ")
        )))
    }
}

fn empty_to_none(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn normalize_paste_text(text: &str) -> String {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    t.replace("\r\n", "\n").replace('\r', "\n")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn decode_base64(raw: &str) -> Result<Vec<u8>, AppCommandError> {
    let trimmed: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if trimmed.len() > 32 * 1024 * 1024 {
        return Err(AppCommandError::invalid_input("encoded file is too large"));
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(trimmed.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(trimmed.as_bytes()))
        .map_err(|_| AppCommandError::invalid_input("invalid base64"))
}

fn safe_filename(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .trim_matches('.');
    let mut cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if cleaned.len() > 200 {
        cleaned.truncate(200);
    }
    if cleaned.is_empty() || cleaned == ".." {
        "upload.bin".into()
    } else {
        cleaned
    }
}

fn paste_filename(title: Option<&str>) -> String {
    let stem = title
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(safe_filename)
        .filter(|s| s != "upload.bin")
        .unwrap_or_else(|| "paste".into());
    if stem.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(ext.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt")
    }) {
        stem
    } else {
        format!("{stem}.md")
    }
}

fn originals_dir(state_root: &Path, source_id: &str) -> PathBuf {
    state_root.join("originals").join(source_id)
}

fn store_original(
    state_root: &Path,
    source_id: &str,
    filename: &str,
    bytes: &[u8],
) -> Result<(), AppCommandError> {
    let dir = originals_dir(state_root, source_id);
    fs::create_dir_all(&dir).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    fs::write(dir.join(filename), bytes).map_err(|e| AppCommandError::io_error(e.to_string()))?;
    Ok(())
}

fn redact_meta(
    title: Option<&str>,
    url: Option<&str>,
    author: Option<&str>,
    personal_role: Option<&str>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
) {
    let mut any = false;
    let map = |v: Option<&str>| {
        v.map(|s| {
            let (t, c) = redact::redact_text(s);
            (t, c)
        })
    };
    let title = map(title);
    let url = map(url);
    let author = map(author);
    let role = map(personal_role);
    any |= title.as_ref().is_some_and(|(_, c)| *c);
    any |= url.as_ref().is_some_and(|(_, c)| *c);
    any |= author.as_ref().is_some_and(|(_, c)| *c);
    any |= role.as_ref().is_some_and(|(_, c)| *c);
    (
        title.map(|(t, _)| t),
        url.map(|(t, _)| t),
        author.map(|(t, _)| t),
        role.map(|(t, _)| t),
        any,
    )
}

fn parse_json_list(raw: &Option<String>) -> Option<Vec<String>> {
    raw.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str(s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::service::wiki_service;
    use crate::db::test_helpers::fresh_in_memory_db;
    use crate::wiki::settings::WikiSettings;
    use std::io::{Cursor, Write};
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    async fn enable_wiki(conn: &DatabaseConnection, vault: &Path) {
        let mut s = WikiSettings::default();
        s.enabled = true;
        s.vault_path = Some(vault.to_string_lossy().to_string());
        settings::save_settings(conn, &s).await.unwrap();
    }

    async fn with_home<F, Fut>(dir: &Path, f: F) -> Fut::Output
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future,
    {
        let home = dir.to_string_lossy().to_string();
        temp_env::async_with_vars(
            [
                ("CODEG_HOME", Some(home.as_str())),
                ("CODEG_DATA_DIR", None::<&str>),
            ],
            f(),
        )
        .await
    }

    fn import_text_params(request_id: &str, text: &str) -> ImportTextParams {
        ImportTextParams {
            request_id: request_id.into(),
            text: text.into(),
            title: Some("Paste note".into()),
            source_url: None,
            author: None,
            material_role: None,
            personal_role: None,
            project_ids: None,
            area_ids: None,
        }
    }

    fn import_file_params(request_id: &str, filename: &str, bytes: &[u8]) -> ImportFilesParams {
        use base64::Engine;
        ImportFilesParams {
            request_id: request_id.into(),
            files: vec![ImportFilePart {
                filename: filename.into(),
                mime: None,
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            }],
            material_role: None,
            personal_role: None,
            title: None,
            source_url: None,
            author: None,
            batch_id: None,
            project_ids: None,
            area_ids: None,
        }
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn pack_zip(files: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
        let buf = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(buf);
        let opts = SimpleFileOptions::default().compression_method(method);
        for (name, data) in files {
            zip.start_file(*name, opts).expect("zip start_file");
            zip.write_all(data).expect("zip write");
        }
        zip.finish().expect("zip finish").into_inner()
    }

    fn heading_docx(heading: &str, para: &str) -> Vec<u8> {
        let heading = xml_escape(heading);
        let para = xml_escape(para);
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading1"/><w:outlineLvl w:val="0"/></w:pPr>
      <w:r><w:t>{heading}</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>{para}</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#
        );
        pack_zip(
            &[("word/document.xml", document.as_bytes())],
            CompressionMethod::Deflated,
        )
    }

    fn escape_pdf_literal(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    }

    fn build_pdf(page_texts: &[&str]) -> Vec<u8> {
        let mut body: Vec<u8> = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0u32];
        let push_obj = |body: &mut Vec<u8>, offsets: &mut Vec<u32>, obj: &str| {
            offsets.push(body.len() as u32);
            body.extend_from_slice(obj.as_bytes());
            if !body.ends_with(&[b'\n']) {
                body.push(b'\n');
            }
        };
        push_obj(
            &mut body,
            &mut offsets,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        let page_count = page_texts.len();
        let first_page_id = 3;
        let first_content_id = 3 + page_count;
        let font_id = 3 + 2 * page_count;
        let kids: String = (0..page_count)
            .map(|i| format!("{} 0 R", first_page_id + i))
            .collect::<Vec<_>>()
            .join(" ");
        let pages_obj =
            format!("2 0 obj\n<< /Type /Pages /Count {page_count} /Kids [{kids}] >>\nendobj\n");
        push_obj(&mut body, &mut offsets, &pages_obj);
        for i in 0..page_count {
            let page_obj = format!(
                "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 {} 0 R >> >> >>\nendobj\n",
                first_page_id + i,
                first_content_id + i,
                font_id
            );
            push_obj(&mut body, &mut offsets, &page_obj);
        }
        for (i, text) in page_texts.iter().enumerate() {
            let escaped = escape_pdf_literal(text);
            let stream = if text.is_empty() {
                "BT ET\n".to_string()
            } else {
                format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET\n")
            };
            let content_obj = format!(
                "{} 0 obj\n<< /Length {} >>\nstream\n{stream}endstream\nendobj\n",
                first_content_id + i,
                stream.len()
            );
            push_obj(&mut body, &mut offsets, &content_obj);
        }
        let font_obj = format!(
            "{font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n"
        );
        push_obj(&mut body, &mut offsets, &font_obj);
        let xref_start = body.len();
        let n_objects = offsets.len();
        body.extend_from_slice(format!("xref\n0 {n_objects}\n").as_bytes());
        body.extend_from_slice(b"0000000000 65535 f \n");
        for off in offsets.iter().skip(1) {
            body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        body.extend_from_slice(
            format!(
                "trailer\n<< /Size {n_objects} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n"
            )
            .as_bytes(),
        );
        body
    }

    fn encrypted_pdf() -> Vec<u8> {
        let mut body: Vec<u8> = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0u32];
        let push_obj = |body: &mut Vec<u8>, offsets: &mut Vec<u32>, obj: &str| {
            offsets.push(body.len() as u32);
            body.extend_from_slice(obj.as_bytes());
        };
        push_obj(
            &mut body,
            &mut offsets,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        push_obj(
            &mut body,
            &mut offsets,
            "2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n",
        );
        push_obj(
            &mut body,
            &mut offsets,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
        );
        push_obj(
            &mut body,
            &mut offsets,
            "4 0 obj\n<< /Filter /Standard /V 1 /R 2 /O <0000000000000000000000000000000000000000000000000000000000000000> /U <0000000000000000000000000000000000000000000000000000000000000000> /P -4 >>\nendobj\n",
        );
        let xref_start = body.len();
        let n_objects = offsets.len();
        body.extend_from_slice(format!("xref\n0 {n_objects}\n").as_bytes());
        body.extend_from_slice(b"0000000000 65535 f \n");
        for off in offsets.iter().skip(1) {
            body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        body.extend_from_slice(
            format!(
                "trailer\n<< /Size {n_objects} /Root 1 0 R /Encrypt 4 0 R >>\nstartxref\n{xref_start}\n%%EOF\n"
            )
            .as_bytes(),
        );
        body
    }

    #[tokio::test]
    async fn paste_markdown_writes_source_and_raw() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let out = import_text(
                &db.conn,
                import_text_params("req-paste-1", "# Hello\n\nBody paragraph."),
            )
            .await
            .unwrap();
            assert!(!out.duplicate);
            assert_eq!(out.source.source_kind, "pasted-text");
            assert_eq!(out.source.eligibility, "ready");
            assert_eq!(out.source.material_role.as_deref(), Some("reference"));
            let raw_path = dir.path().join(out.source.raw_path.as_ref().unwrap());
            let raw = std::fs::read_to_string(&raw_path).unwrap();
            assert!(raw.contains("source_kind: \"pasted-text\""));
            assert!(raw.contains("Hello") || raw.contains("Body paragraph"));
            assert!(raw.contains("locator_kind: paragraph") || raw.contains("## Segments"));
            let originals = dir.path().join("wiki-state/originals").join(&out.source.id);
            assert!(originals.is_dir(), "originals should live under wiki-state");
        })
        .await;
    }

    #[tokio::test]
    async fn duplicate_paste_hash_returns_same_source() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let first = import_text(&db.conn, import_text_params("req-dup-a", "same body\n"))
                .await
                .unwrap();
            let second = import_text(&db.conn, import_text_params("req-dup-b", "same body\n"))
                .await
                .unwrap();
            assert!(second.duplicate);
            assert_eq!(first.source.id, second.source.id);
            let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
                .await
                .unwrap();
            assert_eq!(sources.len(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn same_request_id_retry_returns_same_source() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let first = import_text(&db.conn, import_text_params("req-retry", "retry body"))
                .await
                .unwrap();
            let second = import_text(&db.conn, import_text_params("req-retry", "retry body"))
                .await
                .unwrap();
            assert!(!second.duplicate);
            assert_eq!(first.source.id, second.source.id);
        })
        .await;
    }

    #[tokio::test]
    async fn tiny_pdf_and_docx_write_locators() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let pdf = build_pdf(&["PDF locator page"]);
            let pdf_out = import_files(&db.conn, import_file_params("req-pdf", "note.pdf", &pdf))
                .await
                .unwrap();
            assert_eq!(pdf_out.source.eligibility, "ready");
            assert_eq!(pdf_out.source.format.as_deref(), Some("pdf"));
            let pdf_raw =
                std::fs::read_to_string(dir.path().join(pdf_out.source.raw_path.as_ref().unwrap()))
                    .unwrap();
            assert!(pdf_raw.contains("locator_kind: page"));
            assert!(pdf_raw.contains("PDF locator page"));

            let docx = heading_docx("Overview", "Imported from Word.");
            let docx_out = import_files(
                &db.conn,
                import_file_params("req-docx", "notes.docx", &docx),
            )
            .await
            .unwrap();
            let docx_raw = std::fs::read_to_string(
                dir.path().join(docx_out.source.raw_path.as_ref().unwrap()),
            )
            .unwrap();
            assert!(docx_raw.contains("locator_kind: paragraph"));
            assert!(docx_raw.contains("Imported from Word."));
        })
        .await;
    }

    #[tokio::test]
    async fn encrypted_and_empty_pdf_are_failed_not_ready() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let enc = import_files(
                &db.conn,
                import_file_params("req-enc", "secret.pdf", &encrypted_pdf()),
            )
            .await
            .unwrap();
            assert_eq!(enc.source.eligibility, "failed");
            assert!(enc.source.raw_path.is_none());

            let empty = import_files(
                &db.conn,
                import_file_params("req-empty", "scan.pdf", &build_pdf(&[""])),
            )
            .await
            .unwrap();
            assert_eq!(empty.source.eligibility, "failed");
            assert_ne!(empty.source.eligibility, "ready");
        })
        .await;
    }

    #[tokio::test]
    async fn zip_bomb_rejected() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let zeros = vec![0u8; 80_000];
            let bytes = pack_zip(
                &[("word/document.xml", zeros.as_slice())],
                CompressionMethod::Deflated,
            );
            let err = import_files(
                &db.conn,
                import_file_params("req-bomb", "bomb.docx", &bytes),
            )
            .await
            .unwrap_err();
            assert!(
                err.message.to_lowercase().contains("docx")
                    || err.message.to_lowercase().contains("ratio")
                    || err.message.to_lowercase().contains("rejected"),
                "{}",
                err.message
            );
            let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
                .await
                .unwrap();
            assert!(sources.is_empty());
        })
        .await;
    }

    #[tokio::test]
    async fn oversize_rejected() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let bytes = vec![b'a'; MAX_FILE_BYTES + 1];
            let err = import_files(&db.conn, import_file_params("req-huge", "huge.txt", &bytes))
                .await
                .unwrap_err();
            assert!(
                err.message.contains("exceeds") || err.message.to_lowercase().contains("large"),
                "{}",
                err.message
            );
        })
        .await;
    }

    #[tokio::test]
    async fn partial_pdf_awaits_acceptance_then_ready() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let pdf = build_pdf(&["Visible", ""]);
            let out = import_files(
                &db.conn,
                import_file_params("req-partial", "partial.pdf", &pdf),
            )
            .await
            .unwrap();
            assert_eq!(out.source.eligibility, "awaiting-acceptance");
            assert_eq!(out.source.extraction_status.as_deref(), Some("partial"));
            assert!(!out.source.warnings.is_empty());
            let accepted = accept_extraction(&db.conn, out.source.id.clone())
                .await
                .unwrap();
            assert_eq!(accepted.eligibility, "ready");
        })
        .await;
    }

    #[tokio::test]
    async fn annotation_update_increments_revision_raw_unchanged() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        enable_wiki(&db.conn, dir.path()).await;
        with_home(dir.path(), || async {
            let out = import_text(&db.conn, import_text_params("req-ann", "annotation body"))
                .await
                .unwrap();
            let raw_hash = out.source.raw_hash.clone();
            let rev = out.source.annotation_revision.unwrap_or(0);
            let updated = update_source_annotations(
                &db.conn,
                UpdateAnnotationsParams {
                    source_id: out.source.id.clone(),
                    material_role: Some("own-work".into()),
                    personal_role: Some("author".into()),
                    project_ids: Some(vec!["p1".into()]),
                    area_ids: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(updated.annotation_revision, Some(rev + 1));
            assert_eq!(updated.material_role.as_deref(), Some("own-work"));
            assert_eq!(updated.raw_hash, raw_hash);
            let on_disk =
                std::fs::read_to_string(dir.path().join(out.source.raw_path.as_ref().unwrap()))
                    .unwrap();
            assert!(on_disk.contains("material_role: \"reference\""));
        })
        .await;
    }

    #[tokio::test]
    async fn disabled_settings_reject_import() {
        let db = fresh_in_memory_db().await;
        let dir = tempdir().unwrap();
        let mut s = WikiSettings::default();
        s.enabled = false;
        s.vault_path = Some(dir.path().to_string_lossy().to_string());
        settings::save_settings(&db.conn, &s).await.unwrap();
        with_home(dir.path(), || async {
            let err = import_text(&db.conn, import_text_params("req-off", "should not import"))
                .await
                .unwrap_err();
            assert!(
                err.message.to_lowercase().contains("disabled"),
                "{}",
                err.message
            );
            let sources = wiki_service::list_sources(&db.conn, 10, 0, None)
                .await
                .unwrap();
            assert!(sources.is_empty());
        })
        .await;
    }
}
