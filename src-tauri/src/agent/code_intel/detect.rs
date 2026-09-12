use std::collections::HashSet;
use std::path::Path;

use ignore::WalkBuilder;

use crate::acp::file_system_runtime::FileSystemRuntime;

use super::{CustomLspServer, PresetLsp};

pub const DETECT_MAX_FILES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedLanguage {
    pub server_id: String,
    pub via_manifest: bool,
    pub via_extension: bool,
}

struct Candidate {
    server_id: String,
    manifests: Vec<String>,
    extensions: HashSet<String>,
    via_manifest: bool,
    via_extension: bool,
}

pub fn detect_languages(
    root: &Path,
    fs: &FileSystemRuntime,
    presets: &[PresetLsp],
    custom: &[CustomLspServer],
) -> Vec<DetectedLanguage> {
    let mut candidates: Vec<Candidate> = Vec::with_capacity(presets.len() + custom.len());

    for preset in presets {
        candidates.push(Candidate {
            server_id: preset.id.to_string(),
            manifests: preset.manifests.iter().map(|m| (*m).to_string()).collect(),
            extensions: normalize_extensions(preset.extensions.iter().copied()),
            via_manifest: false,
            via_extension: false,
        });
    }

    for server in custom {
        candidates.push(Candidate {
            server_id: server.id.clone(),
            manifests: server.manifests.clone(),
            extensions: normalize_extensions(server.extensions.iter().map(String::as_str)),
            via_manifest: false,
            via_extension: false,
        });
    }

    for candidate in &mut candidates {
        for manifest in &candidate.manifests {
            let path = root.join(manifest);
            if fs.check_read(&path).is_err() {
                continue;
            }
            if path.is_file() {
                candidate.via_manifest = true;
                break;
            }
        }
    }

    if candidates.iter().all(|c| c.via_manifest) {
        return finish(candidates);
    }

    // Manifest marker paths are handled in step 1; do not re-count them as
    // extension hits (e.g. Cargo.toml must not also activate taplo).
    let manifest_paths: HashSet<_> = candidates
        .iter()
        .flat_map(|c| c.manifests.iter().map(|m| root.join(m)))
        .collect();

    let mut visited = 0usize;
    for entry in walk_files(root) {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        if path == root {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            continue;
        }

        visited += 1;
        if visited > DETECT_MAX_FILES {
            break;
        }

        if manifest_paths.contains(path) {
            continue;
        }

        let Some(ext) = file_extension(path) else {
            continue;
        };

        for candidate in &mut candidates {
            if candidate.via_manifest || candidate.via_extension {
                continue;
            }
            if !candidate.extensions.contains(&ext) {
                continue;
            }
            if fs.check_read(path).is_err() {
                continue;
            }
            candidate.via_extension = true;
        }

        if candidates.iter().all(|c| c.via_manifest || c.via_extension) {
            break;
        }
    }

    finish(candidates)
}

pub fn language_is_present(root: &Path, fs: &FileSystemRuntime, preset: &PresetLsp) -> bool {
    detect_languages(root, fs, std::slice::from_ref(preset), &[])
        .first()
        .is_some_and(|hit| hit.via_manifest || hit.via_extension)
}

fn finish(candidates: Vec<Candidate>) -> Vec<DetectedLanguage> {
    candidates
        .into_iter()
        .filter(|c| c.via_manifest || c.via_extension)
        .map(|c| DetectedLanguage {
            server_id: c.server_id,
            via_manifest: c.via_manifest,
            via_extension: c.via_extension,
        })
        .collect()
}

fn normalize_extensions<'a, I>(exts: I) -> HashSet<String>
where
    I: IntoIterator<Item = &'a str>,
{
    exts.into_iter()
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            if lower.starts_with('.') {
                lower
            } else {
                format!(".{lower}")
            }
        })
        .collect()
}

fn file_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext.to_ascii_lowercase()))
}

fn walk_files(root: &Path) -> ignore::Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
    use crate::agent::code_intel::{detect_languages, preset_lsp_servers, CustomLspServer};

    fn runtime(root: &Path) -> FileSystemRuntime {
        FileSystemRuntime::with_policy(FsAccessPolicy::strict(root))
    }

    #[test]
    fn cargo_toml_detects_rust_analyzer_without_rs_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let hits = detect_languages(dir.path(), &runtime(dir.path()), preset_lsp_servers(), &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].server_id, "rust-analyzer");
        assert!(hits[0].via_manifest);
        assert!(!hits[0].via_extension);
    }

    #[test]
    fn rs_file_detects_rust_analyzer_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        let hits = detect_languages(dir.path(), &runtime(dir.path()), preset_lsp_servers(), &[]);
        assert_eq!(hits[0].server_id, "rust-analyzer");
        assert!(hits[0].via_extension);
    }

    #[test]
    fn empty_dir_detects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let hits = detect_languages(dir.path(), &runtime(dir.path()), preset_lsp_servers(), &[]);
        assert!(hits.is_empty());
    }

    #[test]
    fn custom_server_detects_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "").unwrap();
        let custom = [CustomLspServer {
            id: "elixir-ls".into(),
            language: "Elixir".into(),
            command: "elixir-ls".into(),
            args: vec![],
            extensions: vec![".ex".into()],
            manifests: vec!["mix.exs".into()],
        }];
        let hits = detect_languages(dir.path(), &runtime(dir.path()), preset_lsp_servers(), &custom);
        assert!(hits.iter().any(|h| h.server_id == "elixir-ls" && h.via_manifest));
    }

    #[test]
    fn gitignored_rs_is_not_enough_alone_when_ignore_applies() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "secret.rs\n").unwrap();
        std::fs::write(dir.path().join("secret.rs"), "fn x() {}").unwrap();
        let hits = detect_languages(dir.path(), &runtime(dir.path()), preset_lsp_servers(), &[]);
        assert!(hits.is_empty());
    }
}
