//! Failures for a single-file extract. Limits are never silently truncated.

use std::fmt;

/// Recoverable extract failure. Callers surface these; they are not warnings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    FileTooLarge {
        size: usize,
        max: usize,
    },
    TextTooLong {
        chars: usize,
        max: usize,
    },
    PdfTooManyPages {
        count: u32,
        max: u32,
    },
    EncryptedPdf,
    NoExtractableText,
    DocxLimits(String),
    UnsupportedFormat {
        filename: Option<String>,
        mime_hint: Option<String>,
    },
    InvalidEncoding(String),
    Parse(String),
    Timeout {
        seconds: u64,
    },
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { size, max } => {
                write!(f, "file exceeds {max} bytes (got {size})")
            }
            Self::TextTooLong { chars, max } => {
                write!(f, "normalized text exceeds {max} characters (got {chars})")
            }
            Self::PdfTooManyPages { count, max } => {
                write!(f, "PDF has {count} pages; maximum is {max}")
            }
            Self::EncryptedPdf => write!(f, "PDF is encrypted"),
            Self::NoExtractableText => {
                write!(
                    f,
                    "PDF has no extractable text (empty, scanned, or image-only)"
                )
            }
            Self::DocxLimits(msg) => write!(f, "DOCX archive rejected: {msg}"),
            Self::UnsupportedFormat {
                filename,
                mime_hint,
            } => write!(
                f,
                "unsupported document format (filename: {}, mime: {})",
                filename.as_deref().unwrap_or("-"),
                mime_hint.as_deref().unwrap_or("-")
            ),
            Self::InvalidEncoding(msg) => write!(f, "text encoding error: {msg}"),
            Self::Parse(msg) => write!(f, "failed to parse document: {msg}"),
            Self::Timeout { seconds } => {
                write!(f, "document extraction timed out after {seconds} seconds")
            }
        }
    }
}

impl std::error::Error for ExtractError {}
