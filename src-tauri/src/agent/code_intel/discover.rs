use std::path::{Path, PathBuf};

use super::{sanitize_command_override, CodegraphSettings, SerenaSettings, SERENA_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryKind {
    Global,
    Managed,
    Missing,
    Unconfigured,
    Override,
}

impl DiscoveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Managed => "managed",
            Self::Missing => "missing",
            Self::Unconfigured => "unconfigured",
            Self::Override => "override",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCommand {
    pub kind: DiscoveryKind,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerenaResolution {
    Direct(DiscoveredCommand),
    Uvx { uvx: PathBuf },
    Missing { error: String },
}

pub fn code_tools_managed_root() -> PathBuf {
    crate::paths::codeg_agent_dir().join("code-tools")
}

pub fn managed_provider_binary(root: &Path, provider: &str, file_name: &str) -> PathBuf {
    root.join(provider).join(file_name)
}

pub fn discover_codegraph(
    settings: &CodegraphSettings,
    managed_root: &Path,
    which: &dyn Fn(&str) -> Option<PathBuf>,
) -> DiscoveredCommand {
    let managed = managed_provider_binary(managed_root, "codegraph", "codegraph");
    discover_binary(
        settings.binary_path.as_deref(),
        "codegraph",
        &managed,
        which,
    )
}

pub fn resolve_codegraph_binary(settings: &CodegraphSettings) -> Option<PathBuf> {
    discover_codegraph(settings, &code_tools_managed_root(), &|name| {
        which::which(name).ok()
    })
    .path
}

pub fn resolve_serena(
    settings: &SerenaSettings,
    managed_root: &Path,
    which: &dyn Fn(&str) -> Option<PathBuf>,
) -> SerenaResolution {
    let managed = managed_provider_binary(managed_root, "serena", "serena");
    let found = discover_binary(settings.command.as_deref(), "serena", &managed, which);
    if found.path.is_some() {
        return SerenaResolution::Direct(found);
    }
    if settings.auto_install {
        if let Some(uvx) = which("uvx") {
            return SerenaResolution::Uvx { uvx };
        }
        return SerenaResolution::Missing {
            error: format!("missing runtime: uvx (serena {SERENA_VERSION})"),
        };
    }
    SerenaResolution::Missing {
        error: found
            .error
            .unwrap_or_else(|| "serena command not found".to_string()),
    }
}

pub fn discover_binary(
    override_raw: Option<&str>,
    global_name: &str,
    managed_file: &Path,
    which: &dyn Fn(&str) -> Option<PathBuf>,
) -> DiscoveredCommand {
    let override_raw = sanitize_command_override(override_raw.map(str::to_string));
    if let Some(raw) = override_raw.as_deref() {
        let path = Path::new(raw);
        if path.is_absolute() {
            if path.is_file() {
                return DiscoveredCommand {
                    kind: DiscoveryKind::Override,
                    path: Some(path.to_path_buf()),
                    error: None,
                };
            }
        } else if let Some(found) = which(raw) {
            return DiscoveredCommand {
                kind: DiscoveryKind::Override,
                path: Some(found),
                error: None,
            };
        }
    }
    if let Some(found) = which(global_name) {
        return DiscoveredCommand {
            kind: DiscoveryKind::Global,
            path: Some(found),
            error: None,
        };
    }
    if managed_file.is_file() {
        return DiscoveredCommand {
            kind: DiscoveryKind::Managed,
            path: Some(managed_file.to_path_buf()),
            error: None,
        };
    }
    let error = match override_raw {
        Some(raw) => format!("{global_name} command not found (override `{raw}` is unavailable)"),
        None => format!("{global_name} command not found"),
    };
    DiscoveredCommand {
        kind: DiscoveryKind::Missing,
        path: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::agent::code_intel::{CodegraphSettings, SerenaSettings};

    fn which_map<'a>(hits: &'a [(&'static str, PathBuf)]) -> impl Fn(&str) -> Option<PathBuf> + 'a {
        move |name| {
            hits.iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, path)| path.clone())
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"bin").unwrap();
    }

    #[test]
    fn override_beats_path_and_managed() {
        let dir = tempfile::tempdir().unwrap();
        let override_path = dir.path().join("override bin");
        let global = dir.path().join("global");
        let managed = dir
            .path()
            .join("managed")
            .join("codegraph")
            .join("codegraph");
        touch(&override_path);
        touch(&global);
        touch(&managed);
        let settings = CodegraphSettings {
            enabled: true,
            auto_install: true,
            binary_path: Some(override_path.to_string_lossy().into()),
        };
        let found = discover_codegraph(
            &settings,
            &dir.path().join("managed"),
            &which_map(&[("codegraph", global.clone())]),
        );
        assert_eq!(found.kind, DiscoveryKind::Override);
        assert_eq!(found.path, Some(override_path));
    }

    #[test]
    fn global_beats_managed_directory() {
        let dir = tempfile::tempdir().unwrap();
        let global = dir.path().join("from-path");
        let managed_root = dir.path().join("managed");
        let managed = managed_root.join("codegraph").join("codegraph");
        touch(&global);
        touch(&managed);
        let settings = CodegraphSettings {
            enabled: true,
            auto_install: true,
            binary_path: None,
        };
        let found = discover_codegraph(
            &settings,
            &managed_root,
            &which_map(&[("codegraph", global.clone())]),
        );
        assert_eq!(found.kind, DiscoveryKind::Global);
        assert_eq!(found.path, Some(global));
    }

    #[test]
    fn managed_beats_download_when_path_misses() {
        let dir = tempfile::tempdir().unwrap();
        let managed_root = dir.path().join("managed");
        let managed = managed_root.join("codegraph").join("codegraph");
        touch(&managed);
        let settings = CodegraphSettings::default();
        let found = discover_codegraph(&settings, &managed_root, &which_map(&[]));
        assert_eq!(found.kind, DiscoveryKind::Managed);
        assert_eq!(found.path, Some(managed));
    }

    #[test]
    fn missing_when_nothing_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let found = discover_codegraph(&CodegraphSettings::default(), dir.path(), &which_map(&[]));
        assert_eq!(found.kind, DiscoveryKind::Missing);
        assert!(found.path.is_none());
    }

    #[test]
    fn serena_uses_uvx_only_when_auto_install_and_no_binary() {
        let dir = tempfile::tempdir().unwrap();
        let uvx = dir.path().join("uvx");
        touch(&uvx);
        let mut settings = SerenaSettings::default();
        settings.auto_install = true;
        let resolved = resolve_serena(&settings, dir.path(), &which_map(&[("uvx", uvx.clone())]));
        assert_eq!(resolved, SerenaResolution::Uvx { uvx });

        settings.auto_install = false;
        let resolved = resolve_serena(&settings, dir.path(), &which_map(&[]));
        match resolved {
            SerenaResolution::Missing { error } => {
                assert!(!error.contains("uvx"), "{error}");
            }
            other => panic!("expected missing, got {other:?}"),
        }
    }

    #[test]
    fn serena_missing_runtime_when_uvx_absent() {
        let dir = tempfile::tempdir().unwrap();
        let settings = SerenaSettings::default();
        let resolved = resolve_serena(&settings, dir.path(), &which_map(&[]));
        match resolved {
            SerenaResolution::Missing { error } => {
                assert!(error.contains("missing runtime"), "{error}");
                assert!(error.contains("uvx"), "{error}");
            }
            other => panic!("expected missing runtime, got {other:?}"),
        }
    }

    #[test]
    fn serena_global_binary_beats_uvx() {
        let dir = tempfile::tempdir().unwrap();
        let serena = dir.path().join("serena");
        let uvx = dir.path().join("uvx");
        touch(&serena);
        touch(&uvx);
        let settings = SerenaSettings::default();
        let resolved = resolve_serena(
            &settings,
            dir.path(),
            &which_map(&[("serena", serena.clone()), ("uvx", uvx)]),
        );
        match resolved {
            SerenaResolution::Direct(found) => {
                assert_eq!(found.kind, DiscoveryKind::Global);
                assert_eq!(found.path, Some(serena));
            }
            other => panic!("expected binary, got {other:?}"),
        }
    }
}
