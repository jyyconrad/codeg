//! Markdown: ATX/setext headings become locator paths; other blocks are paragraphs.

use super::error::ExtractError;
use super::limits::Deadline;
use super::text::{decode_text, normalize_paragraph, SegmentBuilder};
use super::{DocFormat, ExtractResult, ExtractionStatus, TextSegment, EXTRACTOR_VERSION};

pub fn extract_markdown(bytes: &[u8], deadline: &Deadline) -> Result<ExtractResult, ExtractError> {
    deadline.check()?;
    let (text, mut warnings, lossy) = decode_text(bytes)?;
    deadline.check()?;
    let segments = segment_markdown(&text, deadline)?;
    let status = if lossy {
        ExtractionStatus::Partial
    } else {
        ExtractionStatus::Complete
    };
    if lossy {
        warnings.push("input was not valid UTF-8; decoded lossily".to_string());
    }
    Ok(ExtractResult {
        text_segments: segments,
        warnings,
        extractor_version: EXTRACTOR_VERSION,
        extraction_status: status,
        page_count: None,
        format: DocFormat::Markdown,
    })
}

pub fn segment_markdown(text: &str, deadline: &Deadline) -> Result<Vec<TextSegment>, ExtractError> {
    let mut builder = SegmentBuilder::new();
    let mut heading_stack: Vec<(u32, String)> = Vec::new();
    let mut para_index = 0u32;
    let mut fence: Option<Fence> = None;
    let mut para_lines: Vec<String> = Vec::new();
    let mut prev_line: Option<String> = None;

    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        deadline.check()?;
        let line = lines[i];

        if let Some(f) = fence {
            para_lines.push(line.to_string());
            if is_fence_close(line, f) {
                flush_code(
                    &mut builder,
                    &mut para_lines,
                    &heading_stack,
                    &mut para_index,
                )?;
                fence = None;
            }
            i += 1;
            continue;
        }

        if let Some(opened) = parse_fence_open(line) {
            flush_paragraph(
                &mut builder,
                &mut para_lines,
                &heading_stack,
                &mut para_index,
                &mut prev_line,
            )?;
            fence = Some(opened);
            para_lines.push(line.to_string());
            i += 1;
            continue;
        }

        if let Some((level, title)) = parse_atx_heading(line) {
            flush_paragraph(
                &mut builder,
                &mut para_lines,
                &heading_stack,
                &mut para_index,
                &mut prev_line,
            )?;
            apply_heading(&mut heading_stack, level, title.clone());
            para_index = 0;
            let path = path_titles(&heading_stack);
            builder.push_paragraph(title, path, 0)?;
            prev_line = None;
            i += 1;
            continue;
        }

        if let Some((level, title)) = parse_setext(&prev_line, line) {
            para_lines.clear();
            apply_heading(&mut heading_stack, level, title.clone());
            para_index = 0;
            let path = path_titles(&heading_stack);
            builder.push_paragraph(title, path, 0)?;
            prev_line = None;
            i += 1;
            continue;
        }

        if line.trim().is_empty() {
            flush_paragraph(
                &mut builder,
                &mut para_lines,
                &heading_stack,
                &mut para_index,
                &mut prev_line,
            )?;
            i += 1;
            continue;
        }

        if let Some(held) = prev_line.take() {
            para_lines.push(held);
        }
        prev_line = Some(line.to_string());
        i += 1;
    }

    if fence.is_some() {
        flush_code(
            &mut builder,
            &mut para_lines,
            &heading_stack,
            &mut para_index,
        )?;
    }
    flush_paragraph(
        &mut builder,
        &mut para_lines,
        &heading_stack,
        &mut para_index,
        &mut prev_line,
    )?;
    Ok(builder.finish())
}

#[derive(Clone, Copy)]
struct Fence {
    marker: u8,
    len: usize,
}

fn parse_fence_open(line: &str) -> Option<Fence> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let len = bytes.iter().take_while(|b| **b == marker).count();
    if len >= 3 {
        Some(Fence { marker, len })
    } else {
        None
    }
}

fn is_fence_close(line: &str, fence: Fence) -> bool {
    let trimmed = line.trim_start().trim_end();
    let bytes = trimmed.as_bytes();
    let len = bytes.iter().take_while(|b| **b == fence.marker).count();
    len >= fence.len && bytes.iter().all(|b| *b == fence.marker)
}

fn parse_atx_heading(line: &str) -> Option<(u32, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let mut title = rest.trim().to_string();
    if let Some(stripped) = title.strip_suffix('#') {
        if stripped.ends_with(char::is_whitespace) || stripped.is_empty() {
            title = stripped.trim_end_matches('#').trim().to_string();
        }
    }
    if title.is_empty() {
        return None;
    }
    Some((hashes as u32, title))
}

fn parse_setext(prev: &Option<String>, line: &str) -> Option<(u32, String)> {
    let prev = prev.as_ref()?;
    let title = prev.trim();
    if title.is_empty() {
        return None;
    }
    let underline = line.trim();
    if underline.is_empty() {
        return None;
    }
    if underline.chars().all(|c| c == '=') {
        Some((1, title.to_string()))
    } else if underline.chars().all(|c| c == '-') && underline.len() >= 2 {
        Some((2, title.to_string()))
    } else {
        None
    }
}

fn apply_heading(stack: &mut Vec<(u32, String)>, level: u32, title: String) {
    stack.retain(|(l, _)| *l < level);
    stack.push((level, title));
}

fn path_titles(stack: &[(u32, String)]) -> Vec<String> {
    stack.iter().map(|(_, t)| t.clone()).collect()
}

fn flush_paragraph(
    builder: &mut SegmentBuilder,
    para_lines: &mut Vec<String>,
    heading_stack: &[(u32, String)],
    para_index: &mut u32,
    prev_line: &mut Option<String>,
) -> Result<(), ExtractError> {
    if let Some(held) = prev_line.take() {
        para_lines.push(held);
    }
    if para_lines.is_empty() {
        return Ok(());
    }
    let joined = para_lines.join("\n");
    para_lines.clear();
    let normalized = normalize_paragraph(&joined);
    if normalized.is_empty() {
        return Ok(());
    }
    *para_index += 1;
    builder.push_paragraph(normalized, path_titles(heading_stack), *para_index)
}

fn flush_code(
    builder: &mut SegmentBuilder,
    para_lines: &mut Vec<String>,
    heading_stack: &[(u32, String)],
    para_index: &mut u32,
) -> Result<(), ExtractError> {
    if para_lines.is_empty() {
        return Ok(());
    }
    let joined = para_lines.join("\n").trim().to_string();
    para_lines.clear();
    if joined.is_empty() {
        return Ok(());
    }
    *para_index += 1;
    builder.push_paragraph(joined, path_titles(heading_stack), *para_index)
}
