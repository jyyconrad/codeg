use std::path::Path;

use calamine::{open_workbook_auto, Data, Range, Reader};

use crate::app_error::AppCommandError;

use super::format::format_cell;
use super::{
    eof_for, window_len, PreviewQuery, SpreadsheetPreviewPage, SpreadsheetSheetInfo, MAX_FILE_BYTES,
};

pub fn preview(
    path: &Path,
    query: &PreviewQuery,
) -> Result<SpreadsheetPreviewPage, AppCommandError> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "xlsx" {
        check_xlsx_zip_limits(path)?;
    }

    let mut workbook = open_workbook_auto(path).map_err(|e| {
        AppCommandError::invalid_input("Failed to open spreadsheet").with_detail(e.to_string())
    })?;

    let sheet_names = workbook.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Err(AppCommandError::invalid_input("Workbook has no sheets"));
    }

    let sheet = match query.sheet.as_deref() {
        Some(name) if !name.is_empty() => {
            if !sheet_names.iter().any(|n| n == name) {
                return Err(AppCommandError::invalid_input("Unknown sheet"));
            }
            name.to_string()
        }
        _ => sheet_names[0].clone(),
    };

    let mut sheets = Vec::with_capacity(sheet_names.len());
    let mut selected: Option<Range<Data>> = None;
    for name in &sheet_names {
        let range = workbook.worksheet_range(name).map_err(|e| {
            AppCommandError::invalid_input("Failed to read sheet").with_detail(e.to_string())
        })?;
        let (height, width) = range.get_size();
        sheets.push(SpreadsheetSheetInfo {
            name: name.clone(),
            row_count: height as u32,
            column_count: width as u32,
        });
        if name == &sheet {
            selected = Some(range);
        }
    }
    let range = selected.ok_or_else(|| AppCommandError::invalid_input("Unknown sheet"))?;
    Ok(page_from_range(path, query, sheets, sheet, &range))
}

fn page_from_range(
    path: &Path,
    query: &PreviewQuery,
    sheets: Vec<SpreadsheetSheetInfo>,
    sheet: String,
    range: &Range<Data>,
) -> SpreadsheetPreviewPage {
    let (height, width) = range.get_size();
    let total_rows = height as u32;
    let total_columns = width as u32;
    let window = window_len(query.col_offset, query.col_limit, total_columns);

    let mut rows_iter = range.rows();
    let header = match rows_iter.next() {
        Some(row) => format_row(row, query.col_offset, window),
        None => Vec::new(),
    };

    let mut skipped = 0u32;
    let mut rows = Vec::new();
    for row in rows_iter {
        if skipped < query.row_offset {
            skipped += 1;
            continue;
        }
        if rows.len() >= query.row_limit as usize {
            break;
        }
        rows.push(format_row(row, query.col_offset, window));
    }

    let eof = eof_for(query.row_offset, rows.len(), total_rows);
    SpreadsheetPreviewPage {
        path: path.to_string_lossy().replace('\\', "/"),
        sheets,
        sheet,
        header,
        row_offset: query.row_offset,
        row_limit: query.row_limit,
        col_offset: query.col_offset,
        col_limit: query.col_limit,
        total_rows,
        total_columns,
        rows,
        eof,
    }
}

fn format_row(row: &[Data], col_offset: u32, window: usize) -> Vec<String> {
    let skip = col_offset as usize;
    let mut out = Vec::with_capacity(window);
    for (i, cell) in row.iter().enumerate() {
        if i < skip {
            continue;
        }
        if out.len() >= window {
            break;
        }
        out.push(format_cell(cell));
    }
    while out.len() < window {
        out.push(String::new());
    }
    out
}

fn check_xlsx_zip_limits(path: &Path) -> Result<(), AppCommandError> {
    let file = std::fs::File::open(path).map_err(AppCommandError::io)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        AppCommandError::invalid_input("Failed to open spreadsheet").with_detail(e.to_string())
    })?;
    let mut total = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| {
            AppCommandError::invalid_input("Failed to open spreadsheet").with_detail(e.to_string())
        })?;
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(
                AppCommandError::invalid_input("Spreadsheet is too large to preview")
                    .with_detail(format!("max_bytes={MAX_FILE_BYTES}")),
            );
        }
        total = total.saturating_add(size);
        if total > MAX_FILE_BYTES {
            return Err(
                AppCommandError::invalid_input("Spreadsheet is too large to preview")
                    .with_detail(format!("max_bytes={MAX_FILE_BYTES}")),
            );
        }
    }
    Ok(())
}
