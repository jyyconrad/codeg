//! Paged spreadsheet preview: open an xlsx/xls/csv on disk and return one
//! window of string cells. The browser never receives the whole workbook.

mod csv_preview;
mod excel;
mod format;

use std::path::Path;

use serde::Serialize;

use crate::app_error::AppCommandError;

pub const MAX_FILE_BYTES: u64 = 50_000_000;
pub const MAX_ROW_LIMIT: u32 = 500;
pub const MAX_COL_LIMIT: u32 = 128;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetSheetInfo {
    pub name: String,
    pub row_count: u32,
    pub column_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpreadsheetPreviewPage {
    pub path: String,
    pub sheets: Vec<SpreadsheetSheetInfo>,
    pub sheet: String,
    pub header: Vec<String>,
    pub row_offset: u32,
    pub row_limit: u32,
    pub col_offset: u32,
    pub col_limit: u32,
    pub total_rows: u32,
    pub total_columns: u32,
    pub rows: Vec<Vec<String>>,
    pub eof: bool,
}

#[derive(Debug, Clone)]
pub struct PreviewQuery {
    pub sheet: Option<String>,
    pub row_offset: u32,
    pub row_limit: u32,
    pub col_offset: u32,
    pub col_limit: u32,
}

impl PreviewQuery {
    pub fn clamped(self) -> Self {
        Self {
            sheet: self.sheet,
            row_offset: self.row_offset,
            row_limit: self.row_limit.clamp(1, MAX_ROW_LIMIT),
            col_offset: self.col_offset,
            col_limit: self.col_limit.clamp(1, MAX_COL_LIMIT),
        }
    }
}

pub fn read_preview(
    path: &Path,
    query: PreviewQuery,
) -> Result<SpreadsheetPreviewPage, AppCommandError> {
    let query = query.clamped();
    let meta = std::fs::metadata(path).map_err(AppCommandError::io)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(
            AppCommandError::invalid_input("Spreadsheet is too large to preview")
                .with_detail(format!("max_bytes={MAX_FILE_BYTES}")),
        );
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => csv_preview::preview(path, &query),
        "xlsx" | "xls" => excel::preview(path, &query),
        _ => Err(AppCommandError::invalid_input(
            "Unsupported spreadsheet type",
        )),
    }
}

pub(crate) fn window_len(col_offset: u32, col_limit: u32, total_columns: u32) -> usize {
    if col_offset >= total_columns {
        return 0;
    }
    col_limit.min(total_columns - col_offset) as usize
}

pub(crate) fn take_window<'a>(
    cells: impl IntoIterator<Item = &'a str>,
    col_offset: u32,
    window: usize,
) -> Vec<String> {
    let skip = col_offset as usize;
    let mut out = Vec::with_capacity(window);
    for (i, cell) in cells.into_iter().enumerate() {
        if i < skip {
            continue;
        }
        if out.len() >= window {
            break;
        }
        out.push(cell.to_string());
    }
    while out.len() < window {
        out.push(String::new());
    }
    out
}

pub(crate) fn eof_for(row_offset: u32, returned: usize, total_rows: u32) -> bool {
    let data_rows = total_rows.saturating_sub(1);
    row_offset as usize + returned >= data_rows as usize
}

#[cfg(test)]
mod tests;
