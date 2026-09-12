//! UTF-8 (lossy) decode and plain-text paragraph segmentation.

use super::error::ExtractError;
use super::limits::{check_text_chars, Deadline, MAX_NORMALIZED_CHARS};
use super::{
    DocFormat, ExtractResult, ExtractionStatus, Locator, LocatorKind, TextSegment,
    EXTRACTOR_VERSION,
};

pub fn extract_plain(bytes: &[u8], deadline: &Deadline) -> Result<ExtractResult, ExtractError> {
    deadline.check()?;
    let (text, mut warnings, lossy) = decode_text(bytes)?;
    deadline.check()?;
    let segments = segment_plain(&text, deadline)?;
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
        format: DocFormat::PlainText,
    })
}

/// Decode document bytes. UTF-8 is preferred; UTF-16 BOM is honoured;
/// anything else is lossy UTF-8 with `lossy = true`.
pub fn decode_text(bytes: &[u8]) -> Result<(String, Vec<String>, bool), ExtractError> {
    let mut warnings = Vec::new();

    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return decode_utf8(rest, warnings, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        warnings.push("decoded UTF-16LE (BOM)".to_string());
        return Ok((decode_utf16(rest, true)?, warnings, false));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        warnings.push("decoded UTF-16BE (BOM)".to_string());
        return Ok((decode_utf16(rest, false)?, warnings, false));
    }
    decode_utf8(bytes, warnings, false)
}

fn decode_utf8(
    bytes: &[u8],
    warnings: Vec<String>,
    _already_lossy: bool,
) -> Result<(String, Vec<String>, bool), ExtractError> {
    match std::str::from_utf8(bytes) {
        Ok(s) => {
            check_text_chars(s.chars().count())?;
            Ok((s.to_string(), warnings, false))
        }
        Err(_) => {
            let lossy = String::from_utf8_lossy(bytes).into_owned();
            check_text_chars(lossy.chars().count())?;
            Ok((lossy, warnings, true))
        }
    }
}

fn decode_utf16(bytes: &[u8], little: bool) -> Result<String, ExtractError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(ExtractError::InvalidEncoding(
            "truncated UTF-16 sequence".to_string(),
        ));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            if little {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    match String::from_utf16(&units) {
        Ok(s) => {
            check_text_chars(s.chars().count())?;
            Ok(s)
        }
        Err(_) => Err(ExtractError::InvalidEncoding(
            "invalid UTF-16 code units".to_string(),
        )),
    }
}

pub fn segment_plain(text: &str, deadline: &Deadline) -> Result<Vec<TextSegment>, ExtractError> {
    let mut builder = SegmentBuilder::new();
    let mut index = 0u32;
    for block in split_blank_lines(text) {
        deadline.check()?;
        let normalized = normalize_paragraph(block);
        if normalized.is_empty() {
            continue;
        }
        index += 1;
        builder.push_paragraph(normalized, Vec::new(), index)?;
    }
    Ok(builder.finish())
}

pub fn split_blank_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i + 1;
            let mut saw_blank = false;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\r') {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'\n' {
                saw_blank = true;
                while j < bytes.len()
                    && (bytes[j] == b'\n'
                        || bytes[j] == b'\r'
                        || bytes[j] == b' '
                        || bytes[j] == b'\t')
                {
                    j += 1;
                }
            }
            if saw_blank {
                out.push(&text[start..i]);
                start = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

pub fn normalize_paragraph(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn paragraph_label(heading_path: &[String], index: u32) -> String {
    if heading_path.is_empty() {
        format!("§{index}")
    } else if index == 0 {
        format!("§{}", heading_path.join(" / "))
    } else {
        format!("§{} / {index}", heading_path.join(" / "))
    }
}

pub struct SegmentBuilder {
    next_id: u32,
    chars: usize,
    segments: Vec<TextSegment>,
}

impl SegmentBuilder {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            chars: 0,
            segments: Vec::new(),
        }
    }

    pub fn push_paragraph(
        &mut self,
        text: String,
        heading_path: Vec<String>,
        index: u32,
    ) -> Result<(), ExtractError> {
        if text.is_empty() {
            return Ok(());
        }
        let added = text.chars().count();
        let total = self.chars.saturating_add(added);
        if total > MAX_NORMALIZED_CHARS {
            return Err(ExtractError::TextTooLong {
                chars: total,
                max: MAX_NORMALIZED_CHARS,
            });
        }
        self.chars = total;
        let label = paragraph_label(&heading_path, index);
        let id = format!("p-{:04}", self.next_id);
        self.next_id += 1;
        self.segments.push(TextSegment {
            id,
            text,
            locator: Locator {
                kind: LocatorKind::Paragraph {
                    heading_path,
                    index,
                },
                label,
            },
        });
        Ok(())
    }

    pub fn push_page(&mut self, number: u32, text: String) -> Result<(), ExtractError> {
        if text.is_empty() {
            return Ok(());
        }
        let added = text.chars().count();
        let total = self.chars.saturating_add(added);
        if total > MAX_NORMALIZED_CHARS {
            return Err(ExtractError::TextTooLong {
                chars: total,
                max: MAX_NORMALIZED_CHARS,
            });
        }
        self.chars = total;
        self.segments.push(TextSegment {
            id: format!("page-{number:03}"),
            text,
            locator: Locator {
                kind: LocatorKind::Page { number },
                label: format!("p.{number}"),
            },
        });
        Ok(())
    }

    pub fn finish(self) -> Vec<TextSegment> {
        self.segments
    }
}

impl Default for SegmentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
