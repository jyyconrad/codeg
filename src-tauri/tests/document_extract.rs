//! Single-file document extract tests (wiki PR2 extract slice).
//!
//! Path-includes the module until the wiki orchestrator adds
//! `pub mod document_extract` to `lib.rs`.

#![allow(dead_code)]

#[path = "../src/document_extract/mod.rs"]
mod document_extract;

use std::io::{Cursor, Write};

use document_extract::{
    extract, DocFormat, ExtractError, ExtractRequest, ExtractionStatus, LocatorKind,
    MAX_DOCX_COMPRESSION_RATIO, MAX_DOCX_ENTRIES, MAX_DOCX_UNCOMPRESSED_BYTES, MAX_FILE_BYTES,
    MAX_NORMALIZED_CHARS, MAX_PDF_PAGES, PARSE_TIMEOUT,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn req<'a>(bytes: &'a [u8], filename: Option<&'a str>) -> ExtractRequest<'a> {
    ExtractRequest {
        bytes,
        filename,
        mime_hint: None,
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn pack_zip(files: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let opts = SimpleFileOptions::default().compression_method(method);
    for (name, data) in files {
        zip.start_file(*name, opts).expect("zip start_file");
        zip.write_all(data).expect("zip write");
    }
    zip.finish().expect("zip finish").into_inner()
}

fn heading_docx(heading: &str, para: &str) -> Vec<u8> {
    let heading = xml_escape(heading);
    let para = xml_escape(para);
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading1"/><w:outlineLvl w:val="0"/></w:pPr>
      <w:r><w:t>{heading}</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:t>{para}</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#
    );
    pack_zip(
        &[("word/document.xml", document.as_bytes())],
        CompressionMethod::Deflated,
    )
}

fn escape_pdf_literal(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn build_pdf(page_texts: &[&str]) -> Vec<u8> {
    let mut body: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0u32];

    let push_obj = |body: &mut Vec<u8>, offsets: &mut Vec<u32>, obj: &str| {
        offsets.push(body.len() as u32);
        body.extend_from_slice(obj.as_bytes());
        if !body.ends_with(&[b'\n']) {
            body.push(b'\n');
        }
    };

    push_obj(
        &mut body,
        &mut offsets,
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );

    let page_count = page_texts.len();
    let first_page_id = 3;
    let first_content_id = 3 + page_count;
    let font_id = 3 + 2 * page_count;

    let kids: String = (0..page_count)
        .map(|i| format!("{} 0 R", first_page_id + i))
        .collect::<Vec<_>>()
        .join(" ");
    let pages_obj =
        format!("2 0 obj\n<< /Type /Pages /Count {page_count} /Kids [{kids}] >>\nendobj\n");
    push_obj(&mut body, &mut offsets, &pages_obj);

    for i in 0..page_count {
        let page_obj = format!(
            "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 {} 0 R >> >> >>\nendobj\n",
            first_page_id + i,
            first_content_id + i,
            font_id
        );
        push_obj(&mut body, &mut offsets, &page_obj);
    }

    for (i, text) in page_texts.iter().enumerate() {
        let escaped = escape_pdf_literal(text);
        let stream = if text.is_empty() {
            "BT ET\n".to_string()
        } else {
            format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET\n")
        };
        let content_obj = format!(
            "{} 0 obj\n<< /Length {} >>\nstream\n{stream}endstream\nendobj\n",
            first_content_id + i,
            stream.len()
        );
        push_obj(&mut body, &mut offsets, &content_obj);
    }

    let font_obj = format!(
        "{font_id} 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>\nendobj\n"
    );
    push_obj(&mut body, &mut offsets, &font_obj);

    let xref_start = body.len();
    let n_objects = offsets.len(); // includes dummy 0
    body.extend_from_slice(format!("xref\n0 {n_objects}\n").as_bytes());
    body.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    body.extend_from_slice(
        format!("trailer\n<< /Size {n_objects} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF\n")
            .as_bytes(),
    );
    body
}

fn encrypted_pdf() -> Vec<u8> {
    let mut body: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0u32];
    let push_obj = |body: &mut Vec<u8>, offsets: &mut Vec<u32>, obj: &str| {
        offsets.push(body.len() as u32);
        body.extend_from_slice(obj.as_bytes());
    };
    push_obj(
        &mut body,
        &mut offsets,
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
    );
    push_obj(
        &mut body,
        &mut offsets,
        "2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n",
    );
    push_obj(
        &mut body,
        &mut offsets,
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n",
    );
    push_obj(
        &mut body,
        &mut offsets,
        "4 0 obj\n<< /Filter /Standard /V 1 /R 2 /O <0000000000000000000000000000000000000000000000000000000000000000> /U <0000000000000000000000000000000000000000000000000000000000000000> /P -4 >>\nendobj\n",
    );
    let xref_start = body.len();
    let n_objects = offsets.len();
    body.extend_from_slice(format!("xref\n0 {n_objects}\n").as_bytes());
    body.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        body.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    body.extend_from_slice(
        format!(
            "trailer\n<< /Size {n_objects} /Root 1 0 R /Encrypt 4 0 R >>\nstartxref\n{xref_start}\n%%EOF\n"
        )
        .as_bytes(),
    );
    body
}

#[test]
fn document_extract_spec_limits_match_wiki_budget() {
    assert_eq!(MAX_FILE_BYTES, 20 * 1024 * 1024);
    assert_eq!(MAX_NORMALIZED_CHARS, 1_000_000);
    assert_eq!(MAX_PDF_PAGES, 500);
    assert_eq!(MAX_DOCX_UNCOMPRESSED_BYTES, 100 * 1024 * 1024);
    assert_eq!(MAX_DOCX_ENTRIES, 10_000);
    assert_eq!(MAX_DOCX_COMPRESSION_RATIO, 100);
    assert_eq!(PARSE_TIMEOUT.as_secs(), 60);
}

#[test]
fn document_extract_utf8_markdown_with_headings() {
    let md = include_str!("fixtures/document_extract/sample.md");
    let result = extract(req(md.as_bytes(), Some("spec.md"))).expect("extract md");
    assert_eq!(result.format, DocFormat::Markdown);
    assert_eq!(result.extraction_status, ExtractionStatus::Complete);
    assert_eq!(
        result.extractor_version,
        document_extract::EXTRACTOR_VERSION
    );
    assert!(result.page_count.is_none());

    let texts: Vec<&str> = result
        .text_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(texts.contains(&"API"), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("Hello wiki")), "{texts:?}");
    assert!(texts.contains(&"Limits"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("Keep extracts bounded")),
        "{texts:?}"
    );

    let api = result
        .text_segments
        .iter()
        .find(|s| s.text == "API")
        .expect("heading API");
    match &api.locator.kind {
        LocatorKind::Paragraph {
            heading_path,
            index,
        } => {
            assert_eq!(heading_path, &["API".to_string()]);
            assert_eq!(*index, 0);
        }
        other => panic!("expected paragraph locator, got {other:?}"),
    }
    assert_eq!(api.locator.label, "§API");

    let hello = result
        .text_segments
        .iter()
        .find(|s| s.text.contains("Hello wiki"))
        .expect("hello para");
    match &hello.locator.kind {
        LocatorKind::Paragraph {
            heading_path,
            index,
        } => {
            assert_eq!(heading_path, &["API".to_string()]);
            assert_eq!(*index, 1);
        }
        other => panic!("expected paragraph locator, got {other:?}"),
    }
    assert_eq!(hello.locator.label, "§API / 1");
    assert!(hello.id.starts_with("p-"));
}

#[test]
fn document_extract_invalid_encoding() {
    let bytes = b"hello \xff world";
    let result = extract(req(bytes, Some("note.txt"))).expect("lossy extract");
    assert_eq!(result.format, DocFormat::PlainText);
    assert_eq!(result.extraction_status, ExtractionStatus::Partial);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.to_ascii_lowercase().contains("utf-8")),
        "warnings: {:?}",
        result.warnings
    );
    let joined = result
        .text_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("hello"), "{joined}");
    assert!(joined.contains("world"), "{joined}");
}

#[test]
fn document_extract_tiny_text_pdf() {
    let pdf = build_pdf(&["Hello Wiki"]);
    let result = extract(req(&pdf, Some("hello.pdf"))).expect("extract pdf");
    assert_eq!(result.format, DocFormat::Pdf);
    assert_eq!(result.extraction_status, ExtractionStatus::Complete);
    assert_eq!(result.page_count, Some(1));
    assert_eq!(result.text_segments.len(), 1);
    assert!(
        result.text_segments[0].text.contains("Hello Wiki"),
        "got {:?}",
        result.text_segments[0].text
    );
    match result.text_segments[0].locator.kind {
        LocatorKind::Page { number } => assert_eq!(number, 1),
        ref other => panic!("expected page locator, got {other:?}"),
    }
    assert_eq!(result.text_segments[0].locator.label, "p.1");
    assert_eq!(result.text_segments[0].id, "page-001");
}

#[test]
fn document_extract_encrypted_pdf_fails() {
    let pdf = encrypted_pdf();
    let err = extract(req(&pdf, Some("secret.pdf"))).expect_err("encrypted pdf");
    assert!(
        matches!(err, ExtractError::EncryptedPdf | ExtractError::Parse(_)),
        "expected encrypted/parse error, got {err:?}"
    );
    assert!(!matches!(
        err,
        ExtractError::FileTooLarge { .. } | ExtractError::TextTooLong { .. }
    ));
}

#[test]
fn document_extract_empty_text_pdf_fails() {
    let pdf = build_pdf(&[""]);
    let err = extract(req(&pdf, Some("scan.pdf"))).expect_err("empty pdf");
    assert!(
        matches!(err, ExtractError::NoExtractableText),
        "expected no extractable text, got {err:?}"
    );
}

#[test]
fn document_extract_tiny_docx_heading_and_paragraph() {
    let bytes = heading_docx("Overview", "Imported from Word.");
    let result = extract(req(&bytes, Some("notes.docx"))).expect("extract docx");
    assert_eq!(result.format, DocFormat::Docx);
    let texts: Vec<&str> = result
        .text_segments
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(texts.iter().any(|t| t.contains("Overview")), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.contains("Imported from Word.")),
        "{texts:?}"
    );
    let heading = result
        .text_segments
        .iter()
        .find(|s| s.text.contains("Overview"))
        .unwrap();
    match &heading.locator.kind {
        LocatorKind::Paragraph {
            heading_path,
            index,
        } => {
            assert_eq!(heading_path.first().map(String::as_str), Some("Overview"));
            assert_eq!(*index, 0);
        }
        other => panic!("expected paragraph locator, got {other:?}"),
    }
}

#[test]
fn document_extract_zip_bomb_ratio_rejected() {
    let zeros = vec![0u8; 80_000];
    let bytes = pack_zip(
        &[("word/document.xml", zeros.as_slice())],
        CompressionMethod::Deflated,
    );
    let err = extract(req(&bytes, Some("bomb.docx"))).expect_err("zip bomb");
    assert!(
        matches!(err, ExtractError::DocxLimits(_)),
        "expected DocxLimits, got {err:?}"
    );
}

#[test]
fn document_extract_zip_too_many_entries_rejected() {
    let buf = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(buf);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for i in 0..=MAX_DOCX_ENTRIES {
        zip.start_file(format!("pad/{i}"), opts).expect("entry");
    }
    let bytes = zip.finish().expect("finish").into_inner();
    let err = extract(req(&bytes, Some("many.docx"))).expect_err("too many entries");
    assert!(
        matches!(err, ExtractError::DocxLimits(_)),
        "expected DocxLimits, got {err:?}"
    );
}

#[test]
fn document_extract_oversize_bytes_rejected() {
    let bytes = vec![b'a'; MAX_FILE_BYTES + 1];
    let err = extract(req(&bytes, Some("huge.txt"))).expect_err("oversize");
    match err {
        ExtractError::FileTooLarge { size, max } => {
            assert_eq!(size, MAX_FILE_BYTES + 1);
            assert_eq!(max, MAX_FILE_BYTES);
        }
        other => panic!("expected FileTooLarge, got {other:?}"),
    }
}

#[test]
fn document_extract_pasted_text_is_plain_text() {
    let result = extract(req(b"just a paste\n\nsecond para", None)).expect("paste");
    assert_eq!(result.format, DocFormat::PlainText);
    assert_eq!(result.extraction_status, ExtractionStatus::Complete);
    assert_eq!(result.text_segments.len(), 2);
    assert_eq!(result.text_segments[0].text, "just a paste");
    assert_eq!(result.text_segments[1].text, "second para");
}

#[test]
fn document_extract_mixed_pdf_empty_page_is_partial() {
    let pdf = build_pdf(&["Visible", ""]);
    let result = extract(req(&pdf, Some("partial.pdf"))).expect("partial pdf");
    assert_eq!(result.format, DocFormat::Pdf);
    assert_eq!(result.extraction_status, ExtractionStatus::Partial);
    assert_eq!(result.page_count, Some(2));
    assert_eq!(result.text_segments.len(), 1);
    assert!(result.text_segments[0].text.contains("Visible"));
    assert!(!result.warnings.is_empty());
}
