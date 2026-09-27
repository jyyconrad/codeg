use std::io::Write;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::SERENA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    ChecksumAbsent,
    ChecksumMismatch { expected: String, actual: String },
    Io(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadManifest {
    pub provider_id: &'static str,
    pub version: &'static str,
    pub sha256: Option<&'static str>,
}

/// Production manifests have no reviewed checksum, so downloads are refused.
pub fn production_download_manifests() -> &'static [DownloadManifest] {
    &[
        DownloadManifest {
            provider_id: "lsp",
            version: "",
            sha256: None,
        },
        DownloadManifest {
            provider_id: "codegraph",
            version: "",
            sha256: None,
        },
        DownloadManifest {
            provider_id: "serena",
            version: SERENA_VERSION,
            sha256: None,
        },
    ]
}

pub fn refuse_manifest_download(manifest: &DownloadManifest) -> Result<(), InstallError> {
    if manifest
        .sha256
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err(InstallError::ChecksumAbsent);
    }
    Ok(())
}

/// Write `bytes` to a temp file beside `dest`, verify SHA-256, then rename.
/// A missing checksum refuses before any file is created. A mismatch deletes
/// the temp file and leaves an existing version in place. Callers pass local
/// bytes; this function does not open a network connection.
pub fn install_verified_bytes(
    bytes: &[u8],
    expected_sha256: Option<&str>,
    dest: &Path,
) -> Result<(), InstallError> {
    let Some(expected_raw) = expected_sha256
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(InstallError::ChecksumAbsent);
    };
    let expected = expected_raw.to_ascii_lowercase();
    let parent = dest
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|err| InstallError::Io(err.to_string()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".code-tools-")
        .suffix(".partial")
        .tempfile_in(parent)
        .map_err(|err| InstallError::Io(err.to_string()))?;
    tmp.write_all(bytes)
        .map_err(|err| InstallError::Io(err.to_string()))?;
    tmp.flush()
        .map_err(|err| InstallError::Io(err.to_string()))?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        let tmp_path = tmp.path().to_path_buf();
        drop(tmp);
        let _ = std::fs::remove_file(tmp_path);
        return Err(InstallError::ChecksumMismatch { expected, actual });
    }
    tmp.persist(dest)
        .map_err(|err| InstallError::Io(err.to_string()))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn production_manifests_refuse_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("tool");
        std::fs::write(&dest, b"old").unwrap();
        for manifest in production_download_manifests() {
            assert!(manifest.sha256.is_none(), "{}", manifest.provider_id);
            assert_eq!(
                refuse_manifest_download(manifest),
                Err(InstallError::ChecksumAbsent)
            );
        }
        assert_eq!(
            install_verified_bytes(b"new", None, &dest),
            Err(InstallError::ChecksumAbsent)
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn checksum_mismatch_deletes_partial_and_keeps_existing_version() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested version").join("codegraph");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"old").unwrap();
        let err = install_verified_bytes(b"hello", Some("deadbeef"), &dest).unwrap_err();
        assert!(
            matches!(err, InstallError::ChecksumMismatch { .. }),
            "{err:?}"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        let leftovers: Vec<_> = walkdir_files(dir.path())
            .into_iter()
            .filter(|path| path != &dest)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn matching_checksum_replaces_destination_from_local_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("my tools").join("serena");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"old").unwrap();
        install_verified_bytes(b"hello", Some(HELLO_SHA), &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
        let leftovers: Vec<_> = walkdir_files(dir.path())
            .into_iter()
            .filter(|path| path != &dest)
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    fn walkdir_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }
}
