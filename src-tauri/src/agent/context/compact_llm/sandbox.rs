//! Compact-llm sandbox: root-limited writes, manifest, integrity checks.
//! Owned exclusively by the sandbox implementer — replace this file in full.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;

use super::{sha256_hex, CompactLlmConfig, CompactLlmError, CompactLlmFileEntry};

/// Root-limited compaction directory. Limits are copied from config at
/// construction; written state is shared with [`CompactLlmWriteFileTool`].
#[derive(Clone)]
pub struct CompactLlmSandbox {
    inner: Arc<Inner>,
}

struct Inner {
    root: PathBuf,
    max_file_bytes: u64,
    max_file_count: usize,
    max_total_bytes: u64,
    written: Mutex<Vec<CompactLlmFileEntry>>,
}

#[derive(Clone)]
pub struct CompactLlmWriteFileTool {
    sandbox: CompactLlmSandbox,
}

#[derive(Debug, Deserialize)]
pub struct CompactLlmWriteFileArgs {
    pub path: String,
    pub content: String,
}

impl CompactLlmSandbox {
    pub fn new(root: impl Into<PathBuf>, config: &CompactLlmConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                root: root.into(),
                max_file_bytes: config.max_file_bytes,
                max_file_count: config.max_file_count,
                max_total_bytes: config.max_total_bytes,
                written: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Reject absolute, `..`, NUL, empty, symlink. Resolve under root without escaping.
    pub fn resolve_relative(&self, relative: &str) -> Result<PathBuf, CompactLlmError> {
        check_relative(relative)?;
        let dest = join_under_root(&self.inner.root, relative)?;
        if !path_is_under_root(&self.inner.root, &dest) {
            return Err(CompactLlmError::sandbox("path escaped sandbox root"));
        }
        reject_symlink_chain(&self.inner.root, &dest)?;
        Ok(dest)
    }

    /// Write UTF-8 text, create parent dirs, flush + sync. Update manifest.
    pub fn write_text(
        &self,
        relative: &str,
        content: &str,
    ) -> Result<CompactLlmFileEntry, CompactLlmError> {
        let dest = self.resolve_relative(relative)?;
        let rel = relative_to_root(&self.inner.root, &dest)?;
        let bytes = content.len() as u64;
        let sha256 = sha256_hex(content.as_bytes());
        let media_type = media_type_for(&rel);
        let entry = CompactLlmFileEntry {
            path: rel,
            bytes,
            sha256,
            media_type,
        };

        let mut written = self.lock_written()?;
        let existing = written.iter().position(|e| e.path == entry.path);
        let old_bytes = existing.map(|i| written[i].bytes).unwrap_or(0);
        if bytes > self.inner.max_file_bytes {
            return Err(CompactLlmError::sandbox(format!(
                "file exceeds max_file_bytes ({} > {})",
                bytes, self.inner.max_file_bytes
            )));
        }
        if existing.is_none() && written.len() >= self.inner.max_file_count {
            return Err(CompactLlmError::sandbox(format!(
                "file count exceeds max_file_count ({})",
                self.inner.max_file_count
            )));
        }
        let total: u64 = written.iter().map(|e| e.bytes).sum();
        let new_total = total.saturating_sub(old_bytes).saturating_add(bytes);
        if new_total > self.inner.max_total_bytes {
            return Err(CompactLlmError::sandbox(format!(
                "total bytes would exceed max_total_bytes ({} > {})",
                new_total, self.inner.max_total_bytes
            )));
        }

        ensure_parents_real(&self.inner.root, &dest)?;
        write_text_file(&dest, content.as_bytes())?;

        if let Some(i) = existing {
            written[i] = entry.clone();
        } else {
            written.push(entry.clone());
        }
        Ok(entry)
    }

    pub fn manifest(&self) -> Vec<CompactLlmFileEntry> {
        self.lock_written()
            .map(|g| g.clone())
            .unwrap_or_else(|_| Vec::new())
    }

    /// Re-read from disk: path relative to root, size, sha256, UTF-8. Return file body.
    pub fn verify_file(&self, entry: &CompactLlmFileEntry) -> Result<String, CompactLlmError> {
        let dest = self.resolve_relative(&entry.path)?;
        let meta = fs::symlink_metadata(&dest).map_err(|err| io_err("stat", &dest, err))?;
        if meta.file_type().is_symlink() {
            return Err(CompactLlmError::sandbox(format!(
                "refusing to read through symlink {}",
                dest.display()
            )));
        }
        let raw = fs::read(&dest).map_err(|err| io_err("read", &dest, err))?;
        let body = String::from_utf8(raw)
            .map_err(|_| CompactLlmError::sandbox(format!("{} is not valid UTF-8", entry.path)))?;
        let bytes = body.len() as u64;
        if bytes != entry.bytes {
            return Err(CompactLlmError::sandbox(format!(
                "size mismatch for {}: disk {bytes}, manifest {}",
                entry.path, entry.bytes
            )));
        }
        let digest = sha256_hex(body.as_bytes());
        if digest != entry.sha256 {
            return Err(CompactLlmError::sandbox(format!(
                "sha256 mismatch for {}",
                entry.path
            )));
        }
        Ok(body)
    }

    pub fn verify_all(&self) -> Result<Vec<(CompactLlmFileEntry, String)>, CompactLlmError> {
        let entries = self.manifest();
        let mut out = Vec::with_capacity(entries.len());
        for entry in entries {
            let body = self.verify_file(&entry)?;
            out.push((entry, body));
        }
        Ok(out)
    }

    pub fn tool(&self) -> CompactLlmWriteFileTool {
        CompactLlmWriteFileTool {
            sandbox: self.clone(),
        }
    }

    fn lock_written(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Vec<CompactLlmFileEntry>>, CompactLlmError> {
        self.inner
            .written
            .lock()
            .map_err(|_| CompactLlmError::sandbox("sandbox lock poisoned"))
    }
}

impl Tool for CompactLlmWriteFileTool {
    const NAME: &'static str = "write_file";
    type Args = CompactLlmWriteFileArgs;
    type Output = String;
    type Error = CompactLlmError;

    fn description(&self) -> String {
        "write a UTF-8 text file relative to this compaction directory only".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path relative to this compaction directory"
                },
                "content": {
                    "type": "string",
                    "description": "Entire UTF-8 file contents"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let entry = self.sandbox.write_text(&args.path, &args.content)?;
        Ok(format!("wrote {} bytes to {}", entry.bytes, entry.path))
    }
}

fn check_relative(relative: &str) -> Result<(), CompactLlmError> {
    if relative.is_empty() {
        return Err(CompactLlmError::sandbox("empty path"));
    }
    if relative.contains('\0') {
        return Err(CompactLlmError::sandbox("path contains NUL"));
    }
    if relative.starts_with('~') {
        return Err(CompactLlmError::sandbox("~ is not allowed"));
    }
    if relative.ends_with('/') || relative.ends_with('\\') {
        return Err(CompactLlmError::sandbox("path must name a file"));
    }
    if is_absolute_input(relative) {
        return Err(CompactLlmError::sandbox("absolute path is not allowed"));
    }
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(CompactLlmError::sandbox("absolute path is not allowed"));
    }
    let mut saw_name = false;
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(name) => {
                if name == "~" {
                    return Err(CompactLlmError::sandbox("~ is not allowed"));
                }
                saw_name = true;
            }
            Component::ParentDir => {
                return Err(CompactLlmError::sandbox(".. is not allowed"));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(CompactLlmError::sandbox("absolute path is not allowed"));
            }
        }
    }
    if !saw_name {
        return Err(CompactLlmError::sandbox("empty path"));
    }
    // On Unix a Windows-style `foo\..\bar` is a single component; still reject `..` segments.
    for segment in relative.split(['/', '\\']) {
        if segment == ".." {
            return Err(CompactLlmError::sandbox(".. is not allowed"));
        }
        if segment == "~" {
            return Err(CompactLlmError::sandbox("~ is not allowed"));
        }
    }
    Ok(())
}

fn is_absolute_input(relative: &str) -> bool {
    let b = relative.as_bytes();
    if b.first().is_some_and(|c| *c == b'/' || *c == b'\\') {
        return true;
    }
    // Windows drive prefix (`C:` / `C:\`) even when parsed on Unix.
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn join_under_root(root: &Path, relative: &str) -> Result<PathBuf, CompactLlmError> {
    let mut dest = root.to_path_buf();
    for c in Path::new(relative).components() {
        match c {
            Component::CurDir => {}
            Component::Normal(name) => dest.push(name),
            _ => return Err(CompactLlmError::sandbox("invalid relative path")),
        }
    }
    Ok(dest)
}

fn path_is_under_root(root: &Path, dest: &Path) -> bool {
    dest.starts_with(root) && dest != root
}

fn relative_to_root(root: &Path, dest: &Path) -> Result<String, CompactLlmError> {
    let rel = dest
        .strip_prefix(root)
        .map_err(|_| CompactLlmError::sandbox("path escaped sandbox root"))?;
    let mut parts = Vec::new();
    for c in rel.components() {
        let Component::Normal(s) = c else {
            return Err(CompactLlmError::sandbox("invalid relative path"));
        };
        let s = s
            .to_str()
            .ok_or_else(|| CompactLlmError::sandbox("path is not valid UTF-8"))?;
        parts.push(s);
    }
    if parts.is_empty() {
        return Err(CompactLlmError::sandbox("path must name a file"));
    }
    Ok(parts.join("/"))
}

fn media_type_for(rel: &str) -> String {
    match Path::new(rel).extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("md") => "text/markdown".to_string(),
        _ => "text/plain".to_string(),
    }
}

fn reject_symlink_chain(root: &Path, dest: &Path) -> Result<(), CompactLlmError> {
    let rel = dest
        .strip_prefix(root)
        .map_err(|_| CompactLlmError::sandbox("path escaped sandbox root"))?;
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(CompactLlmError::sandbox(format!(
                    "refusing to write through symlink {}",
                    cur.display()
                )));
            }
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_err("stat", &cur, err)),
        }
    }
    Ok(())
}

/// Create intermediate directories as real dirs, never via a symlink.
fn ensure_parents_real(root: &Path, dest: &Path) -> Result<(), CompactLlmError> {
    let rel = dest
        .strip_prefix(root)
        .map_err(|_| CompactLlmError::sandbox("path escaped sandbox root"))?;
    let mut comps: Vec<_> = rel.components().collect();
    if comps.is_empty() {
        return Err(CompactLlmError::sandbox("path must name a file"));
    }
    comps.pop();
    fs::create_dir_all(root).map_err(|err| io_err("create sandbox root", root, err))?;
    let mut cur = root.to_path_buf();
    for c in comps {
        cur.push(c);
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(CompactLlmError::sandbox(format!(
                    "refusing to write through symlink {}",
                    cur.display()
                )));
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(CompactLlmError::sandbox(format!(
                    "parent is not a directory: {}",
                    cur.display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&cur).map_err(|e| io_err("create directory", &cur, e))?;
            }
            Err(err) => return Err(io_err("stat", &cur, err)),
        }
    }
    Ok(())
}

fn write_text_file(dest: &Path, content: &[u8]) -> Result<(), CompactLlmError> {
    if let Ok(meta) = fs::symlink_metadata(dest) {
        if meta.file_type().is_symlink() {
            return Err(CompactLlmError::sandbox(format!(
                "refusing to write through symlink {}",
                dest.display()
            )));
        }
    }
    let mut file = File::create(dest).map_err(|err| io_err("create", dest, err))?;
    file.write_all(content)
        .map_err(|err| io_err("write", dest, err))?;
    file.flush().map_err(|err| io_err("flush", dest, err))?;
    file.sync_all().map_err(|err| io_err("sync", dest, err))?;
    Ok(())
}

fn io_err(op: &str, path: &Path, err: io::Error) -> CompactLlmError {
    CompactLlmError::sandbox(format!("failed to {op} {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::tool::Tool;

    fn assert_sandbox<T: std::fmt::Debug>(result: Result<T, CompactLlmError>) {
        assert!(
            matches!(result, Err(CompactLlmError::Sandbox(_))),
            "expected sandbox error, got {result:?}"
        );
    }

    #[test]
    fn accepts_notes_md_and_records_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CompactLlmConfig::disabled();
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        let content = "# notes\n";
        let entry = sandbox.write_text("notes.md", content).expect("write");
        assert_eq!(entry.path, "notes.md");
        assert_eq!(entry.bytes, content.len() as u64);
        assert_eq!(entry.sha256, sha256_hex(content.as_bytes()));
        assert_eq!(entry.media_type, "text/markdown");
        let manifest = sandbox.manifest();
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].path, "notes.md");
        assert_eq!(manifest[0].sha256, entry.sha256);
        assert_eq!(sandbox.verify_file(&entry).unwrap(), content);
        let all = sandbox.verify_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, content);
        assert_eq!(
            fs::read_to_string(dir.path().join("notes.md")).unwrap(),
            content
        );
    }

    #[test]
    fn rejects_escape_absolute_empty_nul_and_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CompactLlmConfig::disabled();
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        assert_sandbox(sandbox.resolve_relative("../x"));
        assert_sandbox(sandbox.write_text("../x", "no"));
        assert_sandbox(sandbox.resolve_relative("/tmp/x"));
        assert_sandbox(sandbox.write_text("/tmp/x", "no"));
        assert_sandbox(sandbox.resolve_relative(""));
        assert_sandbox(sandbox.write_text("", "no"));
        assert_sandbox(sandbox.resolve_relative("foo\0bar"));
        assert_sandbox(sandbox.write_text("foo\0bar", "no"));
        assert_sandbox(sandbox.resolve_relative("foo/../../etc/passwd"));
        assert_sandbox(sandbox.write_text("foo/../../etc/passwd", "no"));
        assert_sandbox(sandbox.resolve_relative("~"));
        assert_sandbox(sandbox.resolve_relative("C:\\Windows\\x"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_dest() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CompactLlmConfig::disabled();
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        let target = dir.path().join("target.txt");
        fs::write(&target, "secret").unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_sandbox(sandbox.write_text("link.md", "overwrite"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "secret");
        assert_eq!(sandbox.manifest().len(), 0);
    }

    #[test]
    fn rejects_file_larger_than_max_file_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CompactLlmConfig::disabled();
        cfg.max_file_bytes = 4;
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        assert_sandbox(sandbox.write_text("a.txt", "12345"));
        assert!(sandbox.manifest().is_empty());
    }

    #[test]
    fn rejects_more_files_than_max_file_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CompactLlmConfig::disabled();
        cfg.max_file_count = 1;
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        sandbox.write_text("a.txt", "x").expect("first file");
        assert_sandbox(sandbox.write_text("b.txt", "y"));
        assert_eq!(sandbox.manifest().len(), 1);
        assert_eq!(sandbox.manifest()[0].path, "a.txt");
    }

    #[test]
    fn rejects_when_total_bytes_would_exceed_max() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = CompactLlmConfig::disabled();
        cfg.max_total_bytes = 5;
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        sandbox.write_text("a.txt", "123").expect("first file");
        assert_sandbox(sandbox.write_text("b.txt", "4567"));
        assert_eq!(sandbox.manifest().len(), 1);
        assert_eq!(sandbox.manifest()[0].bytes, 3);
    }

    #[test]
    fn verify_file_fails_on_tamper_or_non_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CompactLlmConfig::disabled();
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        let entry = sandbox.write_text("notes.md", "hello").expect("write");
        fs::write(dir.path().join("notes.md"), "tampered").unwrap();
        assert_sandbox(sandbox.verify_file(&entry));

        let entry = sandbox.write_text("notes.md", "ok").expect("rewrite");
        fs::write(dir.path().join("notes.md"), [0xff, 0xfe]).unwrap();
        assert_sandbox(sandbox.verify_file(&entry));
    }

    #[tokio::test]
    async fn tool_call_writes_notes_md() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = CompactLlmConfig::disabled();
        let sandbox = CompactLlmSandbox::new(dir.path(), &cfg);
        let tool = sandbox.tool();
        let mut ctx = ToolContext::new();
        let out = tool
            .call(
                &mut ctx,
                CompactLlmWriteFileArgs {
                    path: "notes.md".into(),
                    content: "hello from tool".into(),
                },
            )
            .await
            .expect("tool call");
        assert!(out.contains("notes.md"), "{out}");
        assert_eq!(
            fs::read_to_string(dir.path().join("notes.md")).unwrap(),
            "hello from tool"
        );
        assert_eq!(sandbox.manifest()[0].path, "notes.md");
        assert_eq!(sandbox.manifest()[0].media_type, "text/markdown");
    }
}
