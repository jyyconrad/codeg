//! DOCX: zip + word/document.xml. Headings, paragraphs, linearized table cells.
//! Macros, embeddings, and external targets are not executed or fetched.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;
use zip::ZipArchive;

use super::error::ExtractError;
use super::limits::{
    compression_ratio_exceeded, Deadline, MAX_DOCX_COMPRESSION_RATIO, MAX_DOCX_ENTRIES,
    MAX_DOCX_UNCOMPRESSED_BYTES,
};
use super::text::{normalize_paragraph, SegmentBuilder};
use super::{DocFormat, ExtractResult, ExtractionStatus, EXTRACTOR_VERSION};

pub fn extract_docx(bytes: &[u8], deadline: &Deadline) -> Result<ExtractResult, ExtractError> {
    deadline.check()?;
    let mut archive = open_zip(bytes)?;
    let metas = inspect_archive(&mut archive, deadline)?;

    let doc_index = metas
        .iter()
        .find(|m| is_document_xml(&m.name))
        .map(|m| m.index)
        .ok_or_else(|| ExtractError::Parse("DOCX missing word/document.xml".to_string()))?;

    let styles_index = metas
        .iter()
        .find(|m| is_styles_xml(&m.name))
        .map(|m| m.index);

    deadline.check()?;
    let document_xml = read_entry(&mut archive, doc_index, MAX_DOCX_UNCOMPRESSED_BYTES)?;
    deadline.check()?;
    let style_levels = if let Some(idx) = styles_index {
        let styles_xml = read_entry(&mut archive, idx, MAX_DOCX_UNCOMPRESSED_BYTES)?;
        parse_styles(&styles_xml)
    } else {
        HashMap::new()
    };

    deadline.check()?;
    let mut warnings = Vec::new();
    let segments = parse_document(&document_xml, &style_levels, deadline, &mut warnings)?;

    if segments.is_empty() && warnings.is_empty() {
        warnings.push("DOCX contained no extractable paragraph text".to_string());
    }

    let status = if warnings.iter().any(|w| {
        w.contains("skipped") || w.contains("image") || w.contains("object") || w.contains("table")
    }) {
        ExtractionStatus::Partial
    } else {
        ExtractionStatus::Complete
    };

    Ok(ExtractResult {
        text_segments: segments,
        warnings,
        extractor_version: EXTRACTOR_VERSION,
        extraction_status: status,
        page_count: None,
        format: DocFormat::Docx,
    })
}

/// True when the zip contains `word/document.xml` and passes bomb limits.
pub fn archive_contains_document_xml(bytes: &[u8]) -> Result<bool, ExtractError> {
    let mut archive = match open_zip(bytes) {
        Ok(a) => a,
        Err(ExtractError::Parse(_)) => return Ok(false),
        Err(e) => return Err(e),
    };
    let metas = inspect_archive(&mut archive, &Deadline::new(super::limits::PARSE_TIMEOUT))?;
    Ok(metas.iter().any(|m| is_document_xml(&m.name)))
}

struct EntryMeta {
    index: usize,
    name: String,
}

fn open_zip(bytes: &[u8]) -> Result<ZipArchive<Cursor<&[u8]>>, ExtractError> {
    ZipArchive::new(Cursor::new(bytes)).map_err(|e| ExtractError::Parse(format!("zip: {e}")))
}

fn inspect_archive(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    deadline: &Deadline,
) -> Result<Vec<EntryMeta>, ExtractError> {
    let count = archive.len();
    if count > MAX_DOCX_ENTRIES {
        return Err(ExtractError::DocxLimits(format!(
            "{count} entries; maximum is {MAX_DOCX_ENTRIES}"
        )));
    }

    let mut metas = Vec::with_capacity(count);
    let mut total_uncompressed = 0u64;

    for i in 0..count {
        deadline.check()?;
        let file = archive
            .by_index(i)
            .map_err(|e| ExtractError::Parse(format!("zip entry {i}: {e}")))?;
        if file.encrypted() {
            return Err(ExtractError::DocxLimits(
                "encrypted zip entries are not supported".to_string(),
            ));
        }
        let name = normalize_zip_name(file.name());
        let uncompressed = file.size();
        let compressed = file.compressed_size();
        if compression_ratio_exceeded(uncompressed, compressed) {
            return Err(ExtractError::DocxLimits(format!(
                "entry '{name}' compression ratio exceeds {MAX_DOCX_COMPRESSION_RATIO}x"
            )));
        }
        total_uncompressed = total_uncompressed.saturating_add(uncompressed);
        if total_uncompressed > MAX_DOCX_UNCOMPRESSED_BYTES {
            return Err(ExtractError::DocxLimits(format!(
                "uncompressed size {total_uncompressed} exceeds {MAX_DOCX_UNCOMPRESSED_BYTES} bytes"
            )));
        }
        metas.push(EntryMeta { index: i, name });
    }
    Ok(metas)
}

fn read_entry(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    index: usize,
    cap: u64,
) -> Result<Vec<u8>, ExtractError> {
    let mut file = archive
        .by_index(index)
        .map_err(|e| ExtractError::Parse(format!("zip read: {e}")))?;
    let mut buf = Vec::new();
    let mut limited = (&mut file).take(cap.saturating_add(1));
    limited
        .read_to_end(&mut buf)
        .map_err(|e| ExtractError::Parse(format!("zip inflate: {e}")))?;
    if buf.len() as u64 > cap {
        return Err(ExtractError::DocxLimits(format!(
            "inflated entry exceeds {cap} bytes"
        )));
    }
    Ok(buf)
}

fn normalize_zip_name(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches("./").to_string()
}

fn is_document_xml(name: &str) -> bool {
    name.eq_ignore_ascii_case("word/document.xml")
}

fn is_styles_xml(name: &str) -> bool {
    name.eq_ignore_ascii_case("word/styles.xml")
}

fn attr_val(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).into_owned());
        }
    }
    None
}

fn parse_styles(xml: &[u8]) -> HashMap<String, u32> {
    let mut reader = Reader::from_reader(xml);
    let config = reader.config_mut();
    config.trim_text(false);
    let mut buf = Vec::new();
    let mut map = HashMap::new();
    let mut current_id: Option<String> = None;
    let mut current_level: Option<u32> = None;
    let mut in_style = 0u32;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) => {
                let name = e.local_name();
                if name.as_ref() == b"style" {
                    in_style += 1;
                    current_id = attr_val(&e, b"styleId");
                    current_level = None;
                } else if name.as_ref() == b"outlineLvl" {
                    if let Some(v) = attr_val(&e, b"val") {
                        if let Ok(n) = v.parse::<u32>() {
                            current_level = Some(n + 1);
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"style" && in_style > 0 {
                    in_style -= 1;
                    if let (Some(id), Some(level)) = (current_id.take(), current_level.take()) {
                        map.insert(id, level);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

fn heading_level_from_style(style_id: &str, styles: &HashMap<String, u32>) -> Option<u32> {
    if let Some(level) = styles.get(style_id) {
        return Some(*level);
    }
    let lower = style_id.to_ascii_lowercase();
    let stripped = lower.strip_prefix("heading").unwrap_or(&lower);
    let stripped = stripped.trim();
    if let Ok(n) = stripped.parse::<u32>() {
        if (1..=6).contains(&n) {
            return Some(n);
        }
    }
    None
}

fn parse_document(
    xml: &[u8],
    styles: &HashMap<String, u32>,
    deadline: &Deadline,
    warnings: &mut Vec<String>,
) -> Result<Vec<super::TextSegment>, ExtractError> {
    let mut reader = Reader::from_reader(xml);
    let config = reader.config_mut();
    config.trim_text(false);
    let mut buf = Vec::new();

    let mut builder = SegmentBuilder::new();
    let mut heading_stack: Vec<(u32, String)> = Vec::new();
    let mut para_index = 0u32;

    let mut in_para = 0u32;
    let mut in_table = 0u32;
    let mut in_cell = 0u32;
    let mut in_t = 0u32;
    let mut skip = 0u32;
    let mut para_buf = String::new();
    let mut cell_buf = String::new();
    let mut row_cells: Vec<String> = Vec::new();
    let mut pending_style: Option<String> = None;
    let mut pending_outline: Option<u32> = None;
    let mut saw_table = false;
    let mut skipped_objects = 0u32;
    let mut events = 0u32;

    loop {
        events += 1;
        if events % 4096 == 0 {
            deadline.check()?;
        }
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"p" => {
                        in_para += 1;
                        if in_para == 1 {
                            para_buf.clear();
                            pending_style = None;
                            pending_outline = None;
                        }
                    }
                    b"tbl" => {
                        in_table += 1;
                        saw_table = true;
                    }
                    b"tr" => {
                        if in_table > 0 {
                            row_cells.clear();
                        }
                    }
                    b"tc" => {
                        in_cell += 1;
                        cell_buf.clear();
                    }
                    b"t" => in_t += 1,
                    b"pStyle" => pending_style = attr_val(&e, b"val"),
                    b"outlineLvl" => {
                        if let Some(v) = attr_val(&e, b"val") {
                            if let Ok(n) = v.parse::<u32>() {
                                pending_outline = Some(n + 1);
                            }
                        }
                    }
                    b"drawing" | b"object" | b"pict" | b"oleObject" => {
                        skip += 1;
                        skipped_objects += 1;
                    }
                    b"hyperlink" => {}
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"pStyle" => pending_style = attr_val(&e, b"val"),
                    b"outlineLvl" => {
                        if let Some(v) = attr_val(&e, b"val") {
                            if let Ok(n) = v.parse::<u32>() {
                                pending_outline = Some(n + 1);
                            }
                        }
                    }
                    b"tab" => {
                        if skip == 0 {
                            append_text(in_t, in_para, in_cell, &mut para_buf, &mut cell_buf, "\t");
                        }
                    }
                    b"br" | b"cr" => {
                        if skip == 0 {
                            append_text(in_t, in_para, in_cell, &mut para_buf, &mut cell_buf, "\n");
                        }
                    }
                    b"drawing" | b"object" | b"pict" | b"oleObject" => {
                        skipped_objects += 1;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if skip == 0 && in_t > 0 {
                    let decoded = t.unescape().unwrap_or_default();
                    append_text(
                        in_t,
                        in_para,
                        in_cell,
                        &mut para_buf,
                        &mut cell_buf,
                        decoded.as_ref(),
                    );
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"t" => {
                        if in_t > 0 {
                            in_t -= 1;
                        }
                    }
                    b"p" => {
                        if in_para > 0 {
                            in_para -= 1;
                        }
                        if in_para == 0 {
                            if in_cell > 0 {
                                if !para_buf.is_empty() {
                                    if !cell_buf.is_empty() {
                                        cell_buf.push(' ');
                                    }
                                    cell_buf.push_str(&para_buf);
                                }
                            } else if in_table == 0 {
                                flush_paragraph(
                                    &mut builder,
                                    &mut heading_stack,
                                    &mut para_index,
                                    &para_buf,
                                    pending_style.as_deref(),
                                    pending_outline,
                                    styles,
                                )?;
                            }
                            para_buf.clear();
                            pending_style = None;
                            pending_outline = None;
                        }
                    }
                    b"tc" => {
                        if in_cell > 0 {
                            in_cell -= 1;
                        }
                        row_cells.push(normalize_paragraph(&cell_buf));
                        cell_buf.clear();
                    }
                    b"tr" => {
                        let row = row_cells
                            .iter()
                            .filter(|c| !c.is_empty())
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\t");
                        row_cells.clear();
                        if !row.is_empty() {
                            para_index += 1;
                            builder.push_paragraph(
                                row,
                                heading_stack.iter().map(|(_, t)| t.clone()).collect(),
                                para_index,
                            )?;
                        }
                    }
                    b"tbl" => {
                        if in_table > 0 {
                            in_table -= 1;
                        }
                    }
                    b"drawing" | b"object" | b"pict" | b"oleObject" => {
                        if skip > 0 {
                            skip -= 1;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(ExtractError::Parse(format!("document.xml: {e}")));
            }
            _ => {}
        }
        buf.clear();
    }

    if saw_table {
        warnings.push(
            "table cells were linearized; row/column semantics are not preserved".to_string(),
        );
    }
    if skipped_objects > 0 {
        warnings.push(format!(
            "skipped {skipped_objects} embedded object(s) or drawing(s); images are not interpreted"
        ));
    }

    Ok(builder.finish())
}

fn append_text(
    in_t: u32,
    in_para: u32,
    in_cell: u32,
    para_buf: &mut String,
    cell_buf: &mut String,
    s: &str,
) {
    if in_t == 0 && s != "\t" && s != "\n" {
        return;
    }
    if in_para > 0 {
        para_buf.push_str(s);
    } else if in_cell > 0 {
        cell_buf.push_str(s);
    }
}

fn flush_paragraph(
    builder: &mut SegmentBuilder,
    heading_stack: &mut Vec<(u32, String)>,
    para_index: &mut u32,
    raw: &str,
    style: Option<&str>,
    outline: Option<u32>,
    styles: &HashMap<String, u32>,
) -> Result<(), ExtractError> {
    let text = normalize_paragraph(raw);
    if text.is_empty() {
        return Ok(());
    }
    let level = outline.or_else(|| style.and_then(|id| heading_level_from_style(id, styles)));
    if let Some(level) = level {
        heading_stack.retain(|(l, _)| *l < level);
        heading_stack.push((level, text.clone()));
        *para_index = 0;
        builder.push_paragraph(
            text,
            heading_stack.iter().map(|(_, t)| t.clone()).collect(),
            0,
        )
    } else {
        *para_index += 1;
        builder.push_paragraph(
            text,
            heading_stack.iter().map(|(_, t)| t.clone()).collect(),
            *para_index,
        )
    }
}
