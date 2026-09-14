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

        if let Some((level, title)) = parse_setext(&para_lines, &prev_line, line) {
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
    let trimmed = block_start(line)?;
    let bytes = trimmed.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let len = bytes.iter().take_while(|b| **b == marker).count();
    if len >= 3 && !(marker == b'`' && trimmed[len..].contains('`')) {
        Some(Fence { marker, len })
    } else {
        None
    }
}

fn is_fence_close(line: &str, fence: Fence) -> bool {
    let Some(trimmed) = block_start(line) else {
        return false;
    };
    let trimmed = trimmed.trim_end();
    let bytes = trimmed.as_bytes();
    let len = bytes.iter().take_while(|b| **b == fence.marker).count();
    len >= fence.len && bytes.iter().all(|b| *b == fence.marker)
}

fn parse_atx_heading(line: &str) -> Option<(u32, String)> {
    let trimmed = block_start(line)?;
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
    let mut title = rest.trim();
    let without_closing = title.trim_end_matches('#');
    if without_closing.len() != title.len()
        && (without_closing.is_empty() || without_closing.ends_with(char::is_whitespace))
    {
        title = without_closing.trim_end();
    }
    if title.is_empty() {
        return None;
    }
    Some((hashes as u32, title.to_string()))
}

/// Up to three leading spaces can introduce a Markdown block marker. Four
/// spaces or a tab introduce code, whose markers are literal source text.
fn block_start(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|byte| *byte == b' ').count();
    let rest = &line[spaces..];
    if spaces > 3 || rest.starts_with('\t') {
        None
    } else {
        Some(rest)
    }
}

fn parse_setext(lines: &[String], prev: &Option<String>, line: &str) -> Option<(u32, String)> {
    let prev = prev.as_ref()?;
    let underline = block_start(line)?.trim_end();
    let level = if !underline.is_empty() && underline.chars().all(|c| c == '=') {
        1
    } else if underline.len() >= 2 && underline.chars().all(|c| c == '-') {
        2
    } else {
        return None;
    };
    // Setext headings include the complete preceding paragraph. List markers,
    // quotes and indented code cannot be promoted into document headings.
    if !lines
        .iter()
        .chain(std::iter::once(prev))
        .all(|line| can_be_setext_text(line))
    {
        return None;
    }
    let title = normalize_paragraph(
        &lines
            .iter()
            .chain(std::iter::once(prev))
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
    );
    (!title.is_empty()).then_some((level, title))
}

fn can_be_setext_text(line: &str) -> bool {
    let Some(text) = block_start(line) else {
        return false;
    };
    let text = text.trim_end();
    if text.is_empty() || text.starts_with(['>', '<', '|']) {
        return false;
    }
    if let Some(rest) = text.strip_prefix(['-', '+', '*']) {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return false;
        }
    }
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        if let Some(rest) = text[digits..].strip_prefix(['.', ')']) {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return false;
            }
        }
    }
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    !['-', '*', '_']
        .iter()
        .any(|marker| compact.len() >= 3 && compact.chars().all(|c| c == *marker))
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
    if joined.trim().is_empty() {
        return Ok(());
    }
    *para_index += 1;
    // Whitespace is Markdown syntax: list nesting, table rows, hard breaks
    // and indented code all depend on these original line boundaries.
    builder.push_paragraph(joined, path_titles(heading_stack), *para_index)
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
    let joined = para_lines.join("\n");
    para_lines.clear();
    if joined.trim().is_empty() {
        return Ok(());
    }
    *para_index += 1;
    builder.push_paragraph(joined, path_titles(heading_stack), *para_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_extract::LocatorKind;
    use std::time::Duration;

    fn extract(text: &str) -> Vec<TextSegment> {
        segment_markdown(text, &Deadline::new(Duration::from_secs(2))).unwrap()
    }

    #[test]
    fn lists_tables_and_hard_breaks_keep_markdown_line_boundaries() {
        let input = "# 检查\n\n- 补充空结果样本。\n- 检查数据更新后的连续翻页。\n\n| 输入 | 结果 |\n| --- | --- |\n| 空页 | 结束 |\n\n第一行。  \n第二行。";
        let segments = extract(input);
        assert!(segments
            .iter()
            .any(|s| s.text == "- 补充空结果样本。\n- 检查数据更新后的连续翻页。"));
        assert!(segments
            .iter()
            .any(|s| s.text == "| 输入 | 结果 |\n| --- | --- |\n| 空页 | 结束 |"));
        assert!(segments.iter().any(|s| s.text == "第一行。  \n第二行。"));
    }

    #[test]
    fn nested_lists_keep_item_indentation_and_continuation_lines() {
        let list = concat!(
            "- Parent item\n",
            "  - Nested child\n",
            "    continuation text\n",
            "  1. First ordered item\n",
            "  2. Second ordered item"
        );
        let segments = extract(&format!("# Checklist\n\n{list}"));
        assert!(segments.iter().any(|segment| segment.text == list));
    }

    #[test]
    fn indented_code_is_not_a_document_heading_or_fence() {
        let code =
            "    # literal heading\n    ```\n    let items = [1, 2];\n    ```\n    after = true;";
        let segments = extract(&format!("# Document\n\n{code}"));
        assert!(segments.iter().any(|s| s.text == code));
        assert!(segments.iter().all(|s| match &s.locator.kind {
            LocatorKind::Paragraph { heading_path, .. } =>
                heading_path == &["Document".to_string()],
            _ => false,
        }));
    }

    #[test]
    fn fenced_code_preserves_indentation_and_does_not_close_on_four_space_marker() {
        let code = "   ````md\n   # code heading\n    ````\n   # still code\n   ````";
        let segments = extract(&format!("# Document\n\n{code}\n\nAfter the code."));
        assert!(segments.iter().any(|s| s.text == code));
        assert!(segments.iter().any(|s| s.text == "After the code."));
        assert!(segments.iter().all(|s| match &s.locator.kind {
            LocatorKind::Paragraph { heading_path, .. } =>
                heading_path == &["Document".to_string()],
            _ => false,
        }));
    }

    #[test]
    fn setext_heading_keeps_all_lines_and_rules_after_lists_are_not_headings() {
        let segments =
            extract("First title line\nSecond title line\n---\n\n- first item\n- second item\n---");
        assert_eq!(segments[0].text, "First title line Second title line");
        assert!(segments
            .iter()
            .any(|s| s.text == "- first item\n- second item\n---"));
        assert!(segments
            .iter()
            .all(|s| !s.locator.label.contains("second item")));
    }

    #[test]
    fn atx_closing_hashes_are_not_part_of_the_heading_title() {
        let segments = extract("# API ###\n\nBody.\n\n## C#\n\nNext body.");
        assert_eq!(segments[0].text, "API");
        assert!(segments.iter().any(|s| s.text == "C#"));
    }
}
