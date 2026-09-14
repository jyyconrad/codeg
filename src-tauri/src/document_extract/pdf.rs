//! Text-layer PDF extraction by physical page order. No OCR, no image decode.

use lopdf::Document;

use super::error::ExtractError;
use super::limits::{Deadline, MAX_PDF_PAGES, MAX_PDF_PAGE_STREAM_BYTES};
use super::text::{normalize_paragraph, SegmentBuilder};
use super::{DocFormat, ExtractResult, ExtractionStatus, EXTRACTOR_VERSION};

pub fn extract_pdf(bytes: &[u8], deadline: &Deadline) -> Result<ExtractResult, ExtractError> {
    deadline.check()?;
    let doc = load_pdf(bytes)?;
    if doc.is_encrypted() {
        return Err(ExtractError::EncryptedPdf);
    }

    let pages = doc.get_pages();
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    if page_count == 0 {
        return Err(ExtractError::NoExtractableText);
    }
    if page_count > MAX_PDF_PAGES {
        return Err(ExtractError::PdfTooManyPages {
            count: page_count,
            max: MAX_PDF_PAGES,
        });
    }

    let mut builder = SegmentBuilder::new();
    let mut warnings = Vec::new();
    let mut empty_pages = 0u32;
    let mut failed_pages = 0u32;

    for (number, _) in pages {
        deadline.check()?;
        match page_text(&doc, number) {
            Ok(raw) => {
                let text = normalize_paragraph(&raw);
                if text.is_empty() {
                    empty_pages += 1;
                    warnings.push(format!(
                        "page {number} produced no extractable text (blank, missing, or image-only)"
                    ));
                } else {
                    builder.push_page(number, text)?;
                }
            }
            Err(msg) => {
                failed_pages += 1;
                warnings.push(format!("page {number} could not be parsed: {msg}"));
            }
        }
    }

    let segments = builder.finish();
    if segments.is_empty() {
        return Err(ExtractError::NoExtractableText);
    }

    let status = if empty_pages > 0 || failed_pages > 0 {
        ExtractionStatus::Partial
    } else {
        ExtractionStatus::Complete
    };

    Ok(ExtractResult {
        text_segments: segments,
        warnings,
        extractor_version: EXTRACTOR_VERSION,
        extraction_status: status,
        page_count: Some(page_count),
        format: DocFormat::Pdf,
    })
}

fn load_pdf(bytes: &[u8]) -> Result<Document, ExtractError> {
    match Document::load_mem(bytes) {
        Ok(doc) => Ok(doc),
        Err(e) => {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("encrypt") {
                Err(ExtractError::EncryptedPdf)
            } else {
                Err(ExtractError::Parse(format!("pdf: {msg}")))
            }
        }
    }
}

fn page_text(doc: &Document, number: u32) -> Result<String, String> {
    match doc.extract_text_with_limit(&[number], MAX_PDF_PAGE_STREAM_BYTES) {
        Ok(text) => Ok(text),
        Err(e) => {
            let msg = e.to_string();
            if msg.to_ascii_lowercase().contains("encrypt") {
                return Err(msg);
            }
            // Fall back to the uncapped API for older content streams that
            // trip the limit helper on already-small pages.
            doc.extract_text(&[number]).map_err(|e2| e2.to_string())
        }
    }
}
