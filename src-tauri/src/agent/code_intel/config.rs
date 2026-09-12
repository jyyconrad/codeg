use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CODE_INTEL_FILE_NAME: &str = "code-intel.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeIntelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub codegraph: CodegraphSettings,
    #[serde(default)]
    pub lsp: LspSettings,
}

impl Default for CodeIntelConfig {
    fn default() -> Self {
        default_config()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodegraphSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub binary_path: Option<String>,
}

impl Default for CodegraphSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            binary_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspSettings {
    #[serde(default = "default_true")]
    pub auto_attach: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    #[serde(default = "default_checked_servers")]
    pub checked: Vec<String>,
    #[serde(default)]
    pub custom: Vec<CustomLspServer>,
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            auto_attach: default_true(),
            max_concurrent: default_max_concurrent(),
            checked: default_checked_servers(),
            custom: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomLspServer {
    pub id: String,
    pub language: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub manifests: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetLsp {
    pub id: &'static str,
    pub language: &'static str,
    pub binary: &'static str,
    pub args: &'static [&'static str],
    pub manifests: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub default_checked: bool,
}

pub fn code_intel_path() -> PathBuf {
    crate::paths::codeg_agent_dir().join(CODE_INTEL_FILE_NAME)
}

pub fn default_true() -> bool {
    true
}

pub fn default_max_concurrent() -> u32 {
    2
}

pub fn default_checked_servers() -> Vec<String> {
    preset_lsp_servers()
        .iter()
        .filter(|p| p.default_checked)
        .map(|p| p.id.to_string())
        .collect()
}

pub fn default_config() -> CodeIntelConfig {
    CodeIntelConfig {
        enabled: false,
        codegraph: CodegraphSettings::default(),
        lsp: LspSettings::default(),
    }
}

pub fn load_code_intel_config() -> CodeIntelConfig {
    load_code_intel_config_from(&code_intel_path())
}

pub fn load_code_intel_config_from(path: &Path) -> CodeIntelConfig {
    let Ok(bytes) = fs::read(path) else {
        return default_config();
    };
    let Ok(mut config) = serde_json::from_slice::<CodeIntelConfig>(&bytes) else {
        return default_config();
    };
    config.lsp.max_concurrent = config.lsp.max_concurrent.clamp(1, 8);
    config
}

pub fn save_code_intel_config(config: &CodeIntelConfig) -> std::io::Result<()> {
    save_code_intel_config_to(&code_intel_path(), config)
}

pub fn save_code_intel_config_to(path: &Path, config: &CodeIntelConfig) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)?;
            }
            #[cfg(not(unix))]
            fs::create_dir_all(parent)?;
        }
    }

    let serialized = serde_json::to_string_pretty(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let body = format!("{serialized}\n");

    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        if fs::metadata(path).is_err() {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(path)?;
            return file.write_all(body.as_bytes());
        }
    }

    fs::write(path, &body)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(path)?.permissions().mode();
        if mode & 0o007 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o770))?;
        }
    }

    Ok(())
}

pub fn preset_lsp_servers() -> &'static [PresetLsp] {
    const PRESETS: &[PresetLsp] = &[
        PresetLsp {
            id: "rust-analyzer",
            language: "Rust",
            binary: "rust-analyzer",
            args: &[],
            manifests: &["Cargo.toml", "rust-toolchain.toml"],
            extensions: &[".rs"],
            default_checked: true,
        },
        PresetLsp {
            id: "gopls",
            language: "Go",
            binary: "gopls",
            args: &[],
            manifests: &["go.mod", "go.work"],
            extensions: &[".go"],
            default_checked: true,
        },
        PresetLsp {
            id: "pyright",
            language: "Python",
            binary: "pyright-langserver",
            args: &["--stdio"],
            manifests: &["pyproject.toml", "setup.py", "requirements.txt"],
            extensions: &[".py"],
            default_checked: true,
        },
        PresetLsp {
            id: "typescript",
            language: "TypeScript / JavaScript",
            binary: "typescript-language-server",
            args: &["--stdio"],
            manifests: &["tsconfig.json", "jsconfig.json", "package.json"],
            extensions: &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts"],
            default_checked: true,
        },
        PresetLsp {
            id: "clangd",
            language: "C / C++",
            binary: "clangd",
            args: &[],
            manifests: &["CMakeLists.txt", "compile_commands.json"],
            extensions: &[".c", ".h", ".cpp", ".hpp", ".cc", ".cxx"],
            default_checked: true,
        },
        PresetLsp {
            id: "lua-ls",
            language: "Lua",
            binary: "lua-language-server",
            args: &[],
            manifests: &[".luarc.json"],
            extensions: &[".lua"],
            default_checked: false,
        },
        PresetLsp {
            id: "bash-ls",
            language: "Shell",
            binary: "bash-language-server",
            args: &["start"],
            manifests: &[],
            extensions: &[".sh", ".bash"],
            default_checked: false,
        },
        PresetLsp {
            id: "yaml-ls",
            language: "YAML",
            binary: "yaml-language-server",
            args: &["--stdio"],
            manifests: &[],
            extensions: &[".yaml", ".yml"],
            default_checked: false,
        },
        PresetLsp {
            id: "kotlin-ls",
            language: "Kotlin",
            binary: "kotlin-language-server",
            args: &[],
            manifests: &["build.gradle.kts"],
            extensions: &[".kt", ".kts"],
            default_checked: false,
        },
        PresetLsp {
            id: "zls",
            language: "Zig",
            binary: "zls",
            args: &[],
            manifests: &["build.zig"],
            extensions: &[".zig"],
            default_checked: false,
        },
        PresetLsp {
            id: "taplo",
            language: "TOML",
            binary: "taplo",
            args: &["lsp", "stdio"],
            manifests: &[],
            extensions: &[".toml"],
            default_checked: false,
        },
    ];
    PRESETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_with_five_lsp_checked() {
        let cfg = default_config();
        assert!(!cfg.enabled);
        assert!(cfg.codegraph.enabled);
        assert!(cfg.codegraph.binary_path.is_none());
        assert!(cfg.lsp.auto_attach);
        assert_eq!(cfg.lsp.max_concurrent, 2);
        assert_eq!(
            cfg.lsp.checked,
            vec![
                "rust-analyzer".to_string(),
                "gopls".to_string(),
                "pyright".to_string(),
                "typescript".to_string(),
                "clangd".to_string(),
            ]
        );
        let ids: Vec<_> = preset_lsp_servers().iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "rust-analyzer",
                "gopls",
                "pyright",
                "typescript",
                "clangd",
                "lua-ls",
                "bash-ls",
                "yaml-ls",
                "kotlin-ls",
                "zls",
                "taplo"
            ]
        );
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        assert_eq!(load_code_intel_config_from(&path), default_config());
    }

    #[test]
    fn corrupt_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_code_intel_config_from(&path), default_config());
    }

    #[test]
    fn round_trip_preserves_custom_server_and_binary_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("code-intel.json");
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.codegraph.binary_path = Some("/opt/codegraph".into());
        cfg.lsp.checked = vec!["rust-analyzer".into()];
        cfg.lsp.custom.push(CustomLspServer {
            id: "elixir-ls".into(),
            language: "Elixir".into(),
            command: "elixir-ls".into(),
            args: vec![],
            extensions: vec![".ex".into()],
            manifests: vec!["mix.exs".into()],
        });
        save_code_intel_config_to(&path, &cfg).unwrap();
        assert_eq!(load_code_intel_config_from(&path), cfg);
    }

    #[test]
    fn load_clamps_max_concurrent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(&path, r#"{"enabled":true,"lsp":{"max_concurrent":0}}"#).unwrap();
        assert_eq!(load_code_intel_config_from(&path).lsp.max_concurrent, 1);
        std::fs::write(&path, r#"{"lsp":{"max_concurrent":99}}"#).unwrap();
        assert_eq!(load_code_intel_config_from(&path).lsp.max_concurrent, 8);
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("code-intel.json");
        save_code_intel_config_to(&path, &default_config()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
        let parent_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(parent_mode & 0o077, 0);
    }
}
