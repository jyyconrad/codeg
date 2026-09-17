use std::path::Path;

use crate::app_error::AppCommandError;

use super::{
    eof_for, take_window, window_len, PreviewQuery, SpreadsheetPreviewPage, SpreadsheetSheetInfo,
};

pub fn preview(
    path: &Path,
    query: &PreviewQuery,
) -> Result<SpreadsheetPreviewPage, AppCommandError> {
    let bytes = std::fs::read(path).map_err(AppCommandError::io)?;
    let text = decode_csv_bytes(&bytes)?;
    let sheet_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("csv")
        .to_string();

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut records = Vec::new();
    for rec in reader.records() {
        let rec = rec.map_err(|e| {
            AppCommandError::invalid_input("Failed to parse CSV").with_detail(e.to_string())
        })?;
        records.push(rec.iter().map(|s| s.to_string()).collect::<Vec<String>>());
    }

    if records.is_empty() {
        return Ok(empty_page(path, &sheet_name, query));
    }

    let mut total_columns = 0u32;
    for row in &records {
        total_columns = total_columns.max(row.len() as u32);
    }

    let total_rows = records.len() as u32;
    let header_src = &records[0];
    let window = window_len(query.col_offset, query.col_limit, total_columns);
    let header = take_window(
        header_src.iter().map(|s| s.as_str()),
        query.col_offset,
        window,
    );

    let mut rows = Vec::new();
    let data = &records[1..];
    let start = (query.row_offset as usize).min(data.len());
    let end = (start + query.row_limit as usize).min(data.len());
    for row in &data[start..end] {
        rows.push(take_window(
            row.iter().map(|s| s.as_str()),
            query.col_offset,
            window,
        ));
    }

    let eof = eof_for(query.row_offset, rows.len(), total_rows);
    Ok(SpreadsheetPreviewPage {
        path: path.to_string_lossy().replace('\\', "/"),
        sheets: vec![SpreadsheetSheetInfo {
            name: sheet_name.clone(),
            row_count: total_rows,
            column_count: total_columns,
        }],
        sheet: sheet_name,
        header,
        row_offset: query.row_offset,
        row_limit: query.row_limit,
        col_offset: query.col_offset,
        col_limit: query.col_limit,
        total_rows,
        total_columns,
        rows,
        eof,
    })
}

fn empty_page(path: &Path, sheet_name: &str, query: &PreviewQuery) -> SpreadsheetPreviewPage {
    SpreadsheetPreviewPage {
        path: path.to_string_lossy().replace('\\', "/"),
        sheets: vec![SpreadsheetSheetInfo {
            name: sheet_name.to_string(),
            row_count: 0,
            column_count: 0,
        }],
        sheet: sheet_name.to_string(),
        header: vec![],
        row_offset: query.row_offset,
        row_limit: query.row_limit,
        col_offset: query.col_offset,
        col_limit: query.col_limit,
        total_rows: 0,
        total_columns: 0,
        rows: vec![],
        eof: true,
    }
}

fn decode_csv_bytes(bytes: &[u8]) -> Result<String, AppCommandError> {
    let (utf8, _, utf8_err) = encoding_rs::UTF_8.decode(bytes);
    if !utf8_err {
        return Ok(utf8.into_owned());
    }
    let (gb, _, gb_err) = encoding_rs::GB18030.decode(bytes);
    if !gb_err {
        return Ok(gb.into_owned());
    }
    Err(AppCommandError::invalid_input(
        "CSV is not valid UTF-8 or GB18030",
    ))
}
