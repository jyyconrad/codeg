//! Standalone MD/TXT/PDF/DOCX text extractor for personal-wiki import.
//!
//! Not wired in `lib.rs` (the wiki orchestrator adds `pub mod document_extract`).
//! Integration tests path-include this module.

mod docx;
mod error;
mod limits;
mod markdown;
mod pdf;
mod text;

pub use error::ExtractError;
pub use limits::{
    MAX_DOCX_COMPRESSION_RATIO, MAX_DOCX_ENTRIES, MAX_DOCX_UNCOMPRESSED_BYTES, MAX_FILE_BYTES,
    MAX_NORMALIZED_CHARS, MAX_PDF_PAGES, PARSE_TIMEOUT,
};

pub const EXTRACTOR_VERSION: &str = "codeg-document-extract/1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocFormat {
    Markdown,
    PlainText,
    Pdf,
    Docx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocatorKind {
    Page {
        number: u32,
    },
    Paragraph {
        heading_path: Vec<String>,
        index: u32,
    },
    CharRange {
        start: u32,
        end: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    pub kind: LocatorKind,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSegment {
    pub id: String,
    pub text: String,
    pub locator: Locator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractResult {
    pub text_segments: Vec<TextSegment>,
    pub warnings: Vec<String>,
    pub extractor_version: &'static str,
    pub extraction_status: ExtractionStatus,
    pub page_count: Option<u32>,
    pub format: DocFormat,
}

pub struct ExtractRequest<'a> {
    pub bytes: &'a [u8],
    pub filename: Option<&'a str>,
    pub mime_hint: Option<&'a str>,
}

pub fn extract(req: ExtractRequest<'_>) -> Result<ExtractResult, ExtractError> {
    limits::check_file_size(req.bytes.len())?;
    let deadline = limits::Deadline::new(PARSE_TIMEOUT);
    let format = detect_format(req.filename, req.mime_hint, req.bytes)?;
    deadline.check()?;
    match format {
        DocFormat::Markdown => markdown::extract_markdown(req.bytes, &deadline),
        DocFormat::PlainText => text::extract_plain(req.bytes, &deadline),
        DocFormat::Pdf => pdf::extract_pdf(req.bytes, &deadline),
        DocFormat::Docx => docx::extract_docx(req.bytes, &deadline),
    }
}

fn detect_format(
    filename: Option<&str>,
    mime_hint: Option<&str>,
    bytes: &[u8],
) -> Result<DocFormat, ExtractError> {
    if let Some(name) = filename {
        if let Some(ext) = extension(name) {
            if let Some(fmt) = format_from_ext(&ext) {
                return Ok(fmt);
            }
            if matches!(
                ext.as_str(),
                "pptx"
                    | "xlsx"
                    | "xls"
                    | "doc"
                    | "rtf"
                    | "html"
                    | "htm"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "webp"
                    | "mp3"
                    | "mp4"
                    | "zip"
            ) {
                return Err(ExtractError::UnsupportedFormat {
                    filename: Some(name.to_string()),
                    mime_hint: mime_hint.map(str::to_string),
                });
            }
        }
    }

    if let Some(mime) = mime_hint {
        if let Some(fmt) = format_from_mime(mime) {
            return Ok(fmt);
        }
    }

    if bytes.starts_with(b"%PDF") {
        return Ok(DocFormat::Pdf);
    }
    if is_zip_magic(bytes) {
        return if docx::archive_contains_document_xml(bytes)? {
            Ok(DocFormat::Docx)
        } else {
            Err(ExtractError::UnsupportedFormat {
                filename: filename.map(str::to_string),
                mime_hint: mime_hint.map(str::to_string),
            })
        };
    }

    // Pasted text (no filename) and unknown extensions default to plain text.
    Ok(DocFormat::PlainText)
}

fn extension(filename: &str) -> Option<String> {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let ext = name.rsplit_once('.')?.1;
    if ext.is_empty() || ext == name {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

fn format_from_ext(ext: &str) -> Option<DocFormat> {
    match ext {
        "md" | "markdown" | "mdown" | "mkd" | "mdwn" => Some(DocFormat::Markdown),
        "txt" | "text" => Some(DocFormat::PlainText),
        "pdf" => Some(DocFormat::Pdf),
        "docx" => Some(DocFormat::Docx),
        _ => None,
    }
}

fn format_from_mime(mime: &str) -> Option<DocFormat> {
    let base = mime
        .split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "text/markdown" | "text/x-markdown" => Some(DocFormat::Markdown),
        "text/plain" => Some(DocFormat::PlainText),
        "application/pdf" => Some(DocFormat::Pdf),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some(DocFormat::Docx)
        }
        _ => None,
    }
}

fn is_zip_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
}
