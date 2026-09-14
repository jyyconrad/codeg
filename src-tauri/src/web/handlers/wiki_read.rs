//! 个人 Wiki 阅读与目录刷新接口的 Axum 适配层。
//! HTTP 和桌面共用 wiki/read_model 与 library；处理参数序列化与业务错误映射。
//! 本文件的接口测试覆盖路由到读模型的集成，不另建文件读取或任务状态规则。

use crate::wiki::read_model::*;
use crate::{app_error::AppCommandError, app_state::AppState, wiki::read_model};
use axum::{Extension, Json};
use serde::Deserialize;
use std::sync::Arc;

pub async fn wiki_refresh_library(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<crate::wiki::library::WikiLibrary>, AppCommandError> {
    Ok(Json(
        crate::wiki::library::refresh(&state.db.conn)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}

#[derive(Deserialize)]
pub struct NotesParams {
    pub query: WikiNoteQuery,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteParams {
    pub path: Option<String>,
    #[serde(alias = "note_id")]
    pub note_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceParams {
    #[serde(alias = "source_id")]
    pub source_id: String,
}
#[derive(Deserialize)]
pub struct PageParams {
    pub status: Option<String>,
    pub query: Option<String>,
    pub offset: Option<u64>,
    pub limit: Option<u64>,
}

pub async fn wiki_get_overview(
    Extension(state): Extension<Arc<AppState>>,
) -> Result<Json<WikiOverview>, AppCommandError> {
    Ok(Json(
        read_model::get_overview(&state.db.conn)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}
pub async fn wiki_list_notes(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<NotesParams>,
) -> Result<Json<WikiPage<WikiNoteSummary>>, AppCommandError> {
    Ok(Json(
        read_model::list_notes(&state.db.conn, params.query)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}
pub async fn wiki_read_note(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<NoteParams>,
) -> Result<Json<WikiNoteDetail>, AppCommandError> {
    Ok(Json(
        read_model::read_note(&state.db.conn, params.path, params.note_id)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}
pub async fn wiki_read_source_document(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<SourceParams>,
) -> Result<Json<WikiSourceDocument>, AppCommandError> {
    Ok(Json(
        read_model::read_source_document(&state.db.conn, &params.source_id)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}
pub async fn wiki_list_jobs_page(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<PageParams>,
) -> Result<Json<WikiPage<crate::db::service::wiki_service::WikiJobInfo>>, AppCommandError> {
    Ok(Json(
        read_model::list_jobs(&state.db.conn, params.status, params.offset, params.limit)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}
pub async fn wiki_list_sources_page(
    Extension(state): Extension<Arc<AppState>>,
    Json(params): Json<PageParams>,
) -> Result<Json<WikiPage<crate::db::service::wiki_service::WikiSourceInfo>>, AppCommandError> {
    Ok(Json(
        read_model::list_sources(&state.db.conn, params.query, params.offset, params.limit)
            .await
            .map_err(crate::commands::wiki_read::map_error)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use serde_json::json;

    #[tokio::test]
    async fn http_queries_use_the_same_catalog_and_reject_missing_or_unsafe_documents() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::test_helpers::fresh_in_memory_db().await;
        crate::wiki::vault::initialize_vault(dir.path()).unwrap();
        crate::wiki::settings::save_settings(
            &db.conn,
            &crate::wiki::settings::WikiSettings {
                vault_path: Some(dir.path().to_string_lossy().into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        std::fs::write(dir.path().join("work/records/one.md"),"---\ncodeg_note_id: one\ntitle: HTTP 阅读\ntype: work-record\n---\n# 工作结果\n此正文由两个运行模式共用。").unwrap();
        let state = Arc::new(AppState::new_for_test(db, dir.path().join("test-state")));
        let router = axum::Router::new()
            .route("/wiki_list_notes", post(wiki_list_notes))
            .route("/wiki_read_note", post(wiki_read_note))
            .route(
                "/wiki_read_source_document",
                post(wiki_read_source_document),
            )
            .layer(Extension(state));
        let server = axum_test::TestServer::new(router).unwrap();
        let response = server
            .post("/wiki_list_notes")
            .json(&json!({"query":{"query":"两个运行模式"}}))
            .await;
        response.assert_status_ok();
        assert_eq!(response.json::<serde_json::Value>()["total"], 1);
        let response = server
            .post("/wiki_read_note")
            .json(&json!({"noteId":"one"}))
            .await;
        response.assert_status_ok();
        let note = response.json::<serde_json::Value>();
        assert_eq!(note["note"]["title"], "HTTP 阅读");
        assert!(!note["body"].as_str().unwrap().contains("codeg_note_id"));
        server
            .post("/wiki_read_note")
            .json(&json!({"path":"../private.md"}))
            .await
            .assert_status_bad_request();
        server
            .post("/wiki_read_source_document")
            .json(&json!({"sourceId":"missing"}))
            .await
            .assert_status_not_found();
    }
}
