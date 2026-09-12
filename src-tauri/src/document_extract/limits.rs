//! Product budget from the personal-wiki spec §8.2 / §8.3.
//! Over-limit files fail the extract; they are never truncated to "success".

use std::time::{Duration, Instant};

use super::error::ExtractError;

pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_NORMALIZED_CHARS: usize = 1_000_000;
pub const MAX_PDF_PAGES: u32 = 500;
pub const MAX_DOCX_UNCOMPRESSED_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_DOCX_ENTRIES: usize = 10_000;
pub const MAX_DOCX_COMPRESSION_RATIO: u64 = 100;
/// Cap on decompressed PDF page content streams (zip-bomb analogue).
pub const MAX_PDF_PAGE_STREAM_BYTES: usize = 16 * 1024 * 1024;
pub const PARSE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    start: Instant,
    timeout: Duration,
}

impl Deadline {
    pub fn new(timeout: Duration) -> Self {
        Self {
            start: Instant::now(),
            timeout,
        }
    }

    pub fn check(&self) -> Result<(), ExtractError> {
        if self.start.elapsed() > self.timeout {
            Err(ExtractError::Timeout {
                seconds: self.timeout.as_secs().max(1),
            })
        } else {
            Ok(())
        }
    }
}

pub fn check_file_size(len: usize) -> Result<(), ExtractError> {
    if len > MAX_FILE_BYTES {
        Err(ExtractError::FileTooLarge {
            size: len,
            max: MAX_FILE_BYTES,
        })
    } else {
        Ok(())
    }
}

pub fn check_text_chars(chars: usize) -> Result<(), ExtractError> {
    if chars > MAX_NORMALIZED_CHARS {
        Err(ExtractError::TextTooLong {
            chars,
            max: MAX_NORMALIZED_CHARS,
        })
    } else {
        Ok(())
    }
}

/// Claimed uncompressed / compressed sizes. Directories (both 0) pass.
pub fn compression_ratio_exceeded(uncompressed: u64, compressed: u64) -> bool {
    if uncompressed == 0 {
        return false;
    }
    if compressed == 0 {
        return true;
    }
    uncompressed > compressed.saturating_mul(MAX_DOCX_COMPRESSION_RATIO)
}
