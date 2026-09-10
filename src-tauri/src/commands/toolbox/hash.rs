use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use digest::Digest;
use serde::Serialize;

use super::bytes::encode_hex;
use super::jobs::{cancelled_error, emit_progress, is_cancelled};
use crate::app_error::AppCommandError;
use crate::web::event_bridge::EventEmitter;

const CHUNK: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct HashReport {
    #[serde(rename = "MD5")]
    pub md5: String,
    #[serde(rename = "SHA-1")]
    pub sha1: String,
    #[serde(rename = "SHA-256")]
    pub sha256: String,
    #[serde(rename = "SHA-384")]
    pub sha384: String,
    #[serde(rename = "SHA-512")]
    pub sha512: String,
    #[serde(rename = "SHA3-256")]
    pub sha3_256: String,
    #[serde(rename = "SM3")]
    pub sm3: String,
}

pub fn hash_file_core(
    path: &Path,
    job_id: &str,
    emitter: &EventEmitter,
    cancel: &AtomicBool,
) -> Result<HashReport, AppCommandError> {
    let meta = std::fs::metadata(path).map_err(AppCommandError::io)?;
    if !meta.is_file() {
        return Err(AppCommandError::invalid_input("Path is not a regular file."));
    }
    let total = meta.len();
    let mut file = File::open(path).map_err(AppCommandError::io)?;
    let mut buf = vec![0u8; CHUNK];
    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut sha384 = sha2::Sha384::new();
    let mut sha512 = sha2::Sha512::new();
    let mut sha3 = sha3::Sha3_256::new();
    let mut sm3 = sm3::Sm3::new();
    let mut done = 0u64;
    emit_progress(emitter, job_id, "hash", 0, total);
    loop {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        let n = file.read(&mut buf).map_err(AppCommandError::io)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        md5.update(chunk);
        sha1.update(chunk);
        sha256.update(chunk);
        sha384.update(chunk);
        sha512.update(chunk);
        sha3.update(chunk);
        sm3.update(chunk);
        done += n as u64;
        emit_progress(emitter, job_id, "hash", done, total);
    }
    Ok(HashReport {
        md5: encode_hex(&md5.finalize()),
        sha1: encode_hex(&sha1.finalize()),
        sha256: encode_hex(&sha256.finalize()),
        sha384: encode_hex(&sha384.finalize()),
        sha512: encode_hex(&sha512.finalize()),
        sha3_256: encode_hex(&sha3.finalize()),
        sm3: encode_hex(&sm3.finalize()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::event_bridge::EventEmitter;

    #[test]
    fn hashes_abc_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::write(&path, b"abc").unwrap();
        let cancel = AtomicBool::new(false);
        let report = hash_file_core(&path, "t", &EventEmitter::Noop, &cancel).unwrap();
        assert_eq!(
            report.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(report.md5, "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            report.sm3,
            "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
        );
    }
}
