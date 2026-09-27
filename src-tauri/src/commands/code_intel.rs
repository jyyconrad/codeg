//! Code intelligence settings + workspace status.
//!
//! Persistence lives in `~/.codeg/codeg-agent/code-intel.json` (see
//! `agent::code_intel`). Tauri commands and HTTP twins share the same core
//! helpers so desktop and web mode stay identical. Status is a read: it does
//! not download or spawn providers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
use crate::agent::code_intel::{
    code_tools_managed_root, detect_languages, discover_codegraph, load_code_intel_config,
    normalize_config, preset_lsp_servers, resolve_serena, runtime_snapshot_for,
    save_code_intel_config, CodeIntelConfig, CodeIntelRuntimeSnapshot, DiscoveryKind,
    ProjectCodeIntelSupervisor, ProviderRuntimeSnapshot, SerenaResolution, SERENA_VERSION,
};
use crate::app_error::AppCommandError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodeIntelStatus {
    pub config: CodeIntelConfig,
    pub cwd: Option<String>,
    pub migration_report: Option<String>,
    pub providers: Vec<ProviderStatus>,
    pub lsp_languages: Vec<LspLanguageStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderStatus {
    pub id: String,
    pub configured: bool,
    pub discovery: String,
    pub install: String,
    pub runtime: String,
    pub version: Option<String>,
    pub resolved_command: Option<String>,
    pub last_error: Option<String>,
    pub last_started_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LspLanguageStatus {
    pub language: String,
    pub label: String,
    pub checked: bool,
    pub detected: bool,
    pub provider: String,
}

pub fn get_code_intel_settings_core() -> CodeIntelConfig {
    load_code_intel_config()
}

pub fn set_code_intel_settings_core(
    settings: CodeIntelConfig,
) -> Result<CodeIntelConfig, AppCommandError> {
    let mut settings = normalize_config(settings);
    settings.migration_report = None;
    save_code_intel_config(&settings).map_err(AppCommandError::io)?;
    Ok(load_code_intel_config())
}

pub async fn retry_code_intel_core(cwd: Option<String>) -> CodeIntelStatus {
    if let Some(path) = cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let _ = ProjectCodeIntelSupervisor::retry_failed(Path::new(path)).await;
    }
    get_code_intel_status_core(cwd)
}

pub fn get_code_intel_status_core(cwd: Option<String>) -> CodeIntelStatus {
    let cfg = load_code_intel_config();
    let cwd = cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let runtime = cwd
        .as_deref()
        .filter(|path| path.is_dir())
        .and_then(runtime_snapshot_for);
    build_code_intel_status(
        cfg,
        cwd.as_deref(),
        |name| which::which(name).ok(),
        &code_tools_managed_root(),
        runtime.as_ref(),
    )
}

#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub fn get_code_intel_settings() -> Result<CodeIntelConfig, AppCommandError> {
    Ok(get_code_intel_settings_core())
}

#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub fn set_code_intel_settings(
    settings: CodeIntelConfig,
) -> Result<CodeIntelConfig, AppCommandError> {
    set_code_intel_settings_core(settings)
}

#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub fn get_code_intel_status(cwd: Option<String>) -> Result<CodeIntelStatus, AppCommandError> {
    Ok(get_code_intel_status_core(cwd))
}

#[cfg_attr(feature = "tauri-runtime", tauri::command)]
pub async fn retry_code_intel(cwd: Option<String>) -> Result<CodeIntelStatus, AppCommandError> {
    Ok(retry_code_intel_core(cwd).await)
}

pub(crate) fn build_code_intel_status(
    cfg: CodeIntelConfig,
    cwd: Option<&Path>,
    which: impl Fn(&str) -> Option<PathBuf>,
    managed_root: &Path,
    runtime: Option<&CodeIntelRuntimeSnapshot>,
) -> CodeIntelStatus {
    let cwd_is_dir = cwd.is_some_and(Path::is_dir);
    let detected_ids: HashSet<String> = match cwd {
        Some(root) if cwd_is_dir => {
            let fs = FileSystemRuntime::with_policy(FsAccessPolicy::strict(root));
            detect_languages(root, &fs, preset_lsp_servers())
                .into_iter()
                .map(|hit| hit.server_id)
                .collect()
        }
        _ => HashSet::new(),
    };
    let checked: HashSet<&str> = cfg.lsp.languages.iter().map(String::as_str).collect();
    let lsp_languages = preset_lsp_servers()
        .iter()
        .map(|preset| LspLanguageStatus {
            language: preset.language_key.to_string(),
            label: preset.language.to_string(),
            checked: checked.contains(preset.language_key),
            detected: detected_ids.contains(preset.id),
            provider: "unconfigured".to_string(),
        })
        .collect();
    let migration_report = cfg.migration_report.clone();
    let providers = vec![
        lsp_provider(&cfg, runtime.map(|snap| &snap.lsp)),
        codegraph_provider(
            &cfg,
            managed_root,
            &which,
            runtime.map(|snap| &snap.codegraph),
        ),
        serena_provider(&cfg, managed_root, &which, runtime.map(|snap| &snap.serena)),
    ];
    CodeIntelStatus {
        config: cfg,
        cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
        migration_report,
        providers,
        lsp_languages,
    }
}

fn lsp_provider(
    cfg: &CodeIntelConfig,
    runtime: Option<&ProviderRuntimeSnapshot>,
) -> ProviderStatus {
    let mut status = ProviderStatus {
        id: "lsp".to_string(),
        configured: cfg.lsp.enabled,
        discovery: DiscoveryKind::Unconfigured.as_str().to_string(),
        install: "idle".to_string(),
        runtime: "stopped".to_string(),
        version: None,
        resolved_command: None,
        last_error: None,
        last_started_at: None,
    };
    if let Some(runtime) = runtime {
        apply_runtime(&mut status, runtime);
    }
    status.discovery = DiscoveryKind::Unconfigured.as_str().to_string();
    status.resolved_command = None;
    status.version = None;
    status
}

fn codegraph_provider(
    cfg: &CodeIntelConfig,
    managed_root: &Path,
    which: &impl Fn(&str) -> Option<PathBuf>,
    runtime: Option<&ProviderRuntimeSnapshot>,
) -> ProviderStatus {
    let found = discover_codegraph(&cfg.codegraph, managed_root, which);
    let mut status = ProviderStatus {
        id: "codegraph".to_string(),
        configured: cfg.codegraph.enabled,
        discovery: found.kind.as_str().to_string(),
        install: if found.path.is_some() {
            "installed".to_string()
        } else {
            "idle".to_string()
        },
        runtime: "stopped".to_string(),
        version: None,
        resolved_command: found.path.map(|path| path.to_string_lossy().into_owned()),
        last_error: found.error,
        last_started_at: None,
    };
    if let Some(runtime) = runtime {
        apply_runtime(&mut status, runtime);
    }
    status
}

fn serena_provider(
    cfg: &CodeIntelConfig,
    managed_root: &Path,
    which: &impl Fn(&str) -> Option<PathBuf>,
    runtime: Option<&ProviderRuntimeSnapshot>,
) -> ProviderStatus {
    let mut status = ProviderStatus {
        id: "serena".to_string(),
        configured: cfg.serena.enabled,
        discovery: DiscoveryKind::Missing.as_str().to_string(),
        install: "idle".to_string(),
        runtime: "stopped".to_string(),
        version: None,
        resolved_command: None,
        last_error: None,
        last_started_at: None,
    };
    match resolve_serena(&cfg.serena, managed_root, which) {
        SerenaResolution::Direct(found) => {
            status.discovery = found.kind.as_str().to_string();
            status.install = "installed".to_string();
            status.resolved_command = found.path.map(|path| path.to_string_lossy().into_owned());
        }
        SerenaResolution::Uvx { uvx } => {
            status.discovery = DiscoveryKind::Global.as_str().to_string();
            status.install = "installed".to_string();
            status.version = Some(SERENA_VERSION.to_string());
            status.resolved_command = Some(uvx.to_string_lossy().into_owned());
        }
        SerenaResolution::Missing { error } => {
            status.discovery = DiscoveryKind::Missing.as_str().to_string();
            status.last_error = Some(error);
        }
    }
    if let Some(runtime) = runtime {
        apply_runtime(&mut status, runtime);
    }
    status
}

fn apply_runtime(status: &mut ProviderStatus, runtime: &ProviderRuntimeSnapshot) {
    status.runtime = runtime.runtime.clone();
    if runtime.install != "idle" {
        status.install = runtime.install.clone();
    }
    if runtime.last_error.is_some() {
        status.last_error = runtime.last_error.clone();
    }
    if runtime.last_started_at.is_some() {
        status.last_started_at = runtime.last_started_at.clone();
    }
    if runtime.resolved_command.is_some() {
        status.resolved_command = runtime.resolved_command.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::code_intel::default_config;

    fn which_none(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn status_detects_rust_and_leaves_lsp_unconfigured() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let status = build_code_intel_status(
            default_config(),
            Some(dir.path()),
            which_none,
            dir.path(),
            None,
        );
        let rust = status
            .lsp_languages
            .iter()
            .find(|language| language.language == "rust")
            .expect("rust row");
        assert!(rust.detected);
        assert!(rust.checked);
        assert_eq!(rust.provider, "unconfigured");
        assert_eq!(rust.label, "Rust");
        assert!(status.lsp_languages.iter().any(|language| {
            language.language == "go" && !language.detected && language.provider == "unconfigured"
        }));
        assert_eq!(
            status
                .providers
                .iter()
                .map(|provider| provider.id.as_str())
                .collect::<Vec<_>>(),
            vec!["lsp", "codegraph", "serena"]
        );
        let lsp = &status.providers[0];
        assert!(lsp.configured);
        assert_eq!(lsp.discovery, "unconfigured");
        assert_eq!(lsp.runtime, "stopped");
        assert!(lsp.resolved_command.is_none());
        assert!(status.cwd.is_some());
    }

    #[test]
    fn status_skips_detection_when_cwd_missing() {
        let status =
            build_code_intel_status(default_config(), None, which_none, Path::new("/tmp"), None);
        assert!(status.cwd.is_none());
        assert!(status
            .lsp_languages
            .iter()
            .all(|language| !language.detected));
        assert!(status
            .lsp_languages
            .iter()
            .all(|language| language.provider == "unconfigured"));
    }

    #[test]
    fn status_cwd_not_a_dir_skips_detection() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "").unwrap();
        let status = build_code_intel_status(
            default_config(),
            Some(file.as_path()),
            which_none,
            dir.path(),
            None,
        );
        assert!(status
            .lsp_languages
            .iter()
            .all(|language| !language.detected));
    }

    #[test]
    fn status_reports_override_managed_and_missing_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("spawned");
        let binary = dir.path().join("my tools").join("codegraph");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        #[cfg(unix)]
        {
            std::fs::write(&binary, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(not(unix))]
        std::fs::write(&binary, b"bin").unwrap();
        let managed_root = dir.path().join("managed");
        let managed = managed_root.join("serena").join("serena");
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, b"serena").unwrap();
        let mut cfg = default_config();
        cfg.codegraph.binary_path = Some(binary.to_string_lossy().into_owned());
        cfg.serena.auto_install = false;
        let status = build_code_intel_status(
            cfg,
            Some(dir.path()),
            |name| {
                if name == "codegraph" {
                    Some(PathBuf::from("/usr/bin/codegraph"))
                } else {
                    None
                }
            },
            &managed_root,
            None,
        );
        let codegraph = status
            .providers
            .iter()
            .find(|provider| provider.id == "codegraph")
            .unwrap();
        assert_eq!(codegraph.discovery, "override");
        assert_eq!(
            codegraph.resolved_command.as_deref(),
            Some(binary.to_string_lossy().as_ref())
        );
        assert_eq!(codegraph.install, "installed");
        assert_eq!(codegraph.runtime, "stopped");
        let serena = status
            .providers
            .iter()
            .find(|provider| provider.id == "serena")
            .unwrap();
        assert_eq!(serena.discovery, "managed");
        assert_eq!(serena.install, "installed");
        assert!(!marker.exists());
    }

    #[test]
    fn status_serena_missing_runtime_does_not_download() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = default_config();
        cfg.serena.enabled = true;
        cfg.serena.auto_install = true;
        let status = build_code_intel_status(cfg, Some(dir.path()), which_none, dir.path(), None);
        let serena = status
            .providers
            .iter()
            .find(|provider| provider.id == "serena")
            .unwrap();
        assert!(serena.configured);
        assert_eq!(serena.discovery, "missing");
        assert_eq!(serena.install, "idle");
        assert_eq!(serena.runtime, "stopped");
        let error = serena.last_error.as_deref().unwrap_or("");
        assert!(error.contains("missing runtime"), "{error}");
        assert!(error.contains("uvx"), "{error}");
    }

    #[test]
    fn status_surfaces_migration_report() {
        let mut cfg = default_config();
        cfg.migration_report = Some("Ignored 1 custom LSP server(s).".into());
        let status = build_code_intel_status(cfg, None, which_none, Path::new("/tmp"), None);
        assert_eq!(
            status.migration_report.as_deref(),
            Some("Ignored 1 custom LSP server(s).")
        );
    }

    #[test]
    fn set_then_load_round_trip_via_codeg_home() {
        let tmp = tempfile::tempdir().unwrap();
        temp_env::with_vars([("CODEG_HOME", Some(tmp.path().to_str().unwrap()))], || {
            let mut cfg = default_config();
            cfg.enabled = true;
            cfg.codegraph.binary_path = Some("/opt/codegraph".into());
            cfg.serena.context = "grok".into();
            let loaded = set_code_intel_settings_core(cfg).unwrap();
            assert!(loaded.enabled);
            assert_eq!(
                loaded.codegraph.binary_path.as_deref(),
                Some("/opt/codegraph")
            );
            assert_eq!(loaded.serena.context, "grok");
            assert_eq!(loaded.serena.version, "v1.7.0");
            assert!(loaded.migration_report.is_none());
            assert!(get_code_intel_settings_core().enabled);
        });
    }

    #[test]
    fn http_twins_are_linked() {
        let _ =
            std::any::type_name_of_val(&crate::web::handlers::code_intel::get_code_intel_settings);
        let _ =
            std::any::type_name_of_val(&crate::web::handlers::code_intel::set_code_intel_settings);
        let _ =
            std::any::type_name_of_val(&crate::web::handlers::code_intel::get_code_intel_status);
        let _ = std::any::type_name_of_val(&crate::web::handlers::code_intel::retry_code_intel);
    }

    #[tokio::test]
    async fn retry_without_a_live_lease_does_not_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("spawned-by-retry");
        let status = retry_code_intel_core(Some(dir.path().display().to_string())).await;
        assert!(!marker.exists());
        assert!(status
            .providers
            .iter()
            .all(|provider| provider.runtime == "stopped"));
    }
}
