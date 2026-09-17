use std::path::PathBuf;
use std::time::Duration;

use crate::app_error::AppCommandError;
use crate::commands::folders::{ensure_user_navigable_path, resolve_tree_path, run_file_io};
use crate::spreadsheet_preview::{self, PreviewQuery, SpreadsheetPreviewPage};

const PARSE_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_DELAYS_MS: [u64; 4] = [0, 200, 400, 800];

#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn read_spreadsheet_preview(
    root_path: String,
    path: String,
    sheet: Option<String>,
    row_offset: u32,
    row_limit: u32,
    col_offset: u32,
    col_limit: u32,
) -> Result<SpreadsheetPreviewPage, AppCommandError> {
    let root = PathBuf::from(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(AppCommandError::not_found("Folder does not exist"));
    }
    let target = resolve_tree_path(&root, &path)?;
    ensure_user_navigable_path(&root, &target)?;
    if !target.exists() {
        return Err(AppCommandError::not_found("File does not exist"));
    }
    if !target.is_file() {
        return Err(AppCommandError::invalid_input("Path is not a file"));
    }

    let query = PreviewQuery {
        sheet,
        row_offset,
        row_limit,
        col_offset,
        col_limit,
    };

    let work = run_file_io(move || read_with_retry(&target, query));
    match tokio::time::timeout(PARSE_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_) => Err(AppCommandError::task_execution_failed(
            "Spreadsheet preview timed out",
        )),
    }
}

fn read_with_retry(
    path: &std::path::Path,
    query: PreviewQuery,
) -> Result<SpreadsheetPreviewPage, AppCommandError> {
    let mut last = None;
    for (i, delay) in RETRY_DELAYS_MS.iter().enumerate() {
        if *delay > 0 {
            std::thread::sleep(Duration::from_millis(*delay));
        }
        match spreadsheet_preview::read_preview(path, query.clone()) {
            Ok(page) => return Ok(page),
            Err(err) => {
                let retry = i + 1 < RETRY_DELAYS_MS.len() && is_lock_error(&err);
                last = Some(err);
                if !retry {
                    break;
                }
            }
        }
    }
    Err(last
        .unwrap_or_else(|| AppCommandError::task_execution_failed("Spreadsheet preview failed")))
}

fn is_lock_error(err: &AppCommandError) -> bool {
    matches!(err.code, crate::app_error::AppErrorCode::IoError)
        || err
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("os error 32") || d.contains("Sharing violation"))
}
