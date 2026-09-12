//! Code intelligence settings + workspace status.
//!
//! Persistence lives in `~/.codeg/codeg-agent/code-intel.json` (see
//! `agent::code_intel`). Tauri commands and HTTP twins share the same core
//! helpers so desktop and web mode stay identical.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::acp::file_system_runtime::{FileSystemRuntime, FsAccessPolicy};
use crate::agent::code_intel::{
    advertised_mcp_tools, codegraph_has_index, detect_languages, load_code_intel_config,
    preset_lsp_servers, resolve_codegraph_binary, save_code_intel_config, CodeIntelConfig,
    CodeIntelMcpToolStatus,
};
use crate::app_error::AppCommandError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeIntelStatus {
    pub config: CodeIntelConfig,
    pub codegraph_binary: Option<String>,
    pub codegraph_indexed: bool,
    pub cwd: Option<String>,
    pub lsp_servers: Vec<LspServerStatus>,
    #[serde(default)]
    pub mcp_tools: Vec<CodeIntelMcpToolStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspServerStatus {
    pub id: String,
    pub language: String,
    pub binary: String,
    pub binary_on_path: bool,
    pub checked: bool,
    pub language_detected: bool,
    pub default_checked: bool,
    pub custom: bool,
}

pub fn get_code_intel_settings_core() -> CodeIntelConfig {
    load_code_intel_config()
}

pub fn set_code_intel_settings_core(
    mut settings: CodeIntelConfig,
) -> Result<CodeIntelConfig, AppCommandError> {
    settings.lsp.max_concurrent = settings.lsp.max_concurrent.clamp(1, 8);
    save_code_intel_config(&settings).map_err(AppCommandError::io)?;
    Ok(load_code_intel_config())
}

pub fn get_code_intel_status_core(cwd: Option<String>) -> CodeIntelStatus {
    let cfg = load_code_intel_config();
    let cwd = cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    build_code_intel_status(cfg, cwd.as_deref(), |name| which::which(name).is_ok())
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

pub(crate) fn build_code_intel_status(
    cfg: CodeIntelConfig,
    cwd: Option<&Path>,
    on_path: impl Fn(&str) -> bool,
) -> CodeIntelStatus {
    let cwd_is_dir = cwd.is_some_and(Path::is_dir);
    let codegraph_indexed = cwd.is_some_and(|path| path.is_dir() && codegraph_has_index(path));
    let detected_ids: HashSet<String> = match cwd {
        Some(root) if cwd_is_dir => {
            let fs = FileSystemRuntime::with_policy(FsAccessPolicy::strict(root));
            detect_languages(root, &fs, preset_lsp_servers(), &cfg.lsp.custom)
                .into_iter()
                .map(|hit| hit.server_id)
                .collect()
        }
        _ => HashSet::new(),
    };

    let checked: HashSet<&str> = cfg.lsp.checked.iter().map(String::as_str).collect();
    let mut lsp_servers = Vec::with_capacity(preset_lsp_servers().len() + cfg.lsp.custom.len());
    for preset in preset_lsp_servers() {
        lsp_servers.push(LspServerStatus {
            id: preset.id.to_string(),
            language: preset.language.to_string(),
            binary: preset.binary.to_string(),
            binary_on_path: on_path(preset.binary),
            checked: checked.contains(preset.id),
            language_detected: detected_ids.contains(preset.id),
            default_checked: preset.default_checked,
            custom: false,
        });
    }
    for server in &cfg.lsp.custom {
        lsp_servers.push(LspServerStatus {
            id: server.id.clone(),
            language: server.language.clone(),
            binary: server.command.clone(),
            binary_on_path: on_path(&server.command),
            checked: checked.contains(server.id.as_str()),
            language_detected: detected_ids.contains(&server.id),
            default_checked: false,
            custom: true,
        });
    }

    let mcp_tools = advertised_mcp_tools(&cfg);
    CodeIntelStatus {
        codegraph_binary: resolve_codegraph_binary(&cfg.codegraph)
            .map(|path| path.to_string_lossy().into_owned()),
        codegraph_indexed,
        cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
        lsp_servers,
        mcp_tools,
        config: cfg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::code_intel::{default_config, CustomLspServer};

    #[test]
    fn status_detects_rust_from_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let cfg = default_config();
        let status = build_code_intel_status(cfg, Some(dir.path()), |_| true);
        let ra = status
            .lsp_servers
            .iter()
            .find(|s| s.id == "rust-analyzer")
            .expect("rust-analyzer preset");
        assert!(ra.language_detected);
        assert!(ra.binary_on_path);
        assert!(ra.checked);
        assert!(ra.default_checked);
        assert!(!ra.custom);
        assert_eq!(ra.binary, "rust-analyzer");
        assert!(!status.codegraph_indexed);
        assert!(status
            .mcp_tools
            .iter()
            .any(|tool| tool.name == "goToDefinition"));
        assert!(status
            .mcp_tools
            .iter()
            .any(|tool| tool.name == "codegraph_explore"));
        assert!(status.mcp_tools.iter().all(|tool| !tool.advertised));
    }

    #[test]
    fn status_skips_detection_when_cwd_missing() {
        let cfg = default_config();
        let status = build_code_intel_status(cfg, None, |_| true);
        assert!(!status.codegraph_indexed);
        assert!(status.cwd.is_none());
        assert!(status.lsp_servers.iter().all(|s| !s.language_detected));
        assert!(status
            .lsp_servers
            .iter()
            .any(|s| s.id == "rust-analyzer" && s.binary_on_path));
    }

    #[test]
    fn status_cwd_not_a_dir_skips_index_and_detection() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-dir");
        std::fs::write(&file, "").unwrap();
        let status = build_code_intel_status(default_config(), Some(file.as_path()), |_| true);
        assert!(!status.codegraph_indexed);
        assert!(status.lsp_servers.iter().all(|s| !s.language_detected));
    }

    #[test]
    fn status_indexed_when_codegraph_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".codegraph")).unwrap();
        let status = build_code_intel_status(default_config(), Some(dir.path()), |_| false);
        assert!(status.codegraph_indexed);
        assert_eq!(
            status.cwd.as_deref(),
            Some(dir.path().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn status_includes_custom_servers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("mix.exs"), "").unwrap();
        let mut cfg = default_config();
        cfg.lsp.custom.push(CustomLspServer {
            id: "elixir-ls".into(),
            language: "Elixir".into(),
            command: "elixir-ls".into(),
            args: vec![],
            extensions: vec![".ex".into()],
            manifests: vec!["mix.exs".into()],
        });
        cfg.lsp.checked.push("elixir-ls".into());
        let status = build_code_intel_status(cfg, Some(dir.path()), |name| name == "elixir-ls");
        let custom = status
            .lsp_servers
            .iter()
            .find(|s| s.id == "elixir-ls")
            .expect("custom server");
        assert!(custom.custom);
        assert!(custom.language_detected);
        assert!(custom.binary_on_path);
        assert!(custom.checked);
        assert!(!custom.default_checked);
        assert_eq!(custom.binary, "elixir-ls");
        let ra = status
            .lsp_servers
            .iter()
            .find(|s| s.id == "rust-analyzer")
            .expect("preset remains");
        assert!(!ra.binary_on_path);
        assert!(!ra.custom);
    }

    #[test]
    fn set_then_load_round_trip_via_codeg_home() {
        let tmp = tempfile::tempdir().unwrap();
        temp_env::with_vars([("CODEG_HOME", Some(tmp.path().to_str().unwrap()))], || {
            let mut cfg = default_config();
            cfg.enabled = true;
            cfg.codegraph.binary_path = Some("/opt/codegraph".into());
            let loaded = set_code_intel_settings_core(cfg.clone()).unwrap();
            assert!(loaded.enabled);
            assert_eq!(
                loaded.codegraph.binary_path.as_deref(),
                Some("/opt/codegraph")
            );
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
    }
}
