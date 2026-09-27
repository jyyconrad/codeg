use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CODE_INTEL_FILE_NAME: &str = "code-intel.json";
pub const SERENA_VERSION: &str = "v1.7.0";
pub const SERENA_UVX_FROM: &str = "git+https://github.com/oraios/serena@v1.7.0";

pub const SERENA_CONTEXTS: &[&str] = &[
    "agent",
    "antigravity",
    "chatgpt",
    "claude-code",
    "codebuddy",
    "codex",
    "copilot-cli",
    "desktop-app",
    "grok",
    "ide",
    "jb-ai-assistant",
    "jb-copilot-plugin",
    "junie",
    "oaicompat-agent",
    "vscode",
];

pub const SERENA_MODES: &[&str] = &[
    "benchmark",
    "editing",
    "interactive",
    "no-memories",
    "no-onboarding",
    "onboarding",
    "one-shot",
    "planning",
    "query-projects",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeIntelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub lsp: LspSettings,
    #[serde(default)]
    pub codegraph: CodegraphSettings,
    #[serde(default)]
    pub serena: SerenaSettings,
    /// In-memory only. Old `lsp.custom` is never executed and is not written back.
    #[serde(skip)]
    pub migration_report: Option<String>,
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
    #[serde(default = "default_true")]
    pub auto_install: bool,
    #[serde(default)]
    pub binary_path: Option<String>,
}

impl Default for CodegraphSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            auto_install: default_true(),
            binary_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LspSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_language_keys")]
    pub languages: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    #[serde(default = "default_true")]
    pub auto_install: bool,
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            languages: default_language_keys(),
            max_concurrent: default_max_concurrent(),
            auto_install: default_true(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerenaSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_install: bool,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default = "default_serena_version")]
    pub version: String,
    #[serde(default = "default_serena_context")]
    pub context: String,
    #[serde(default = "default_serena_modes")]
    pub modes: Vec<String>,
}

impl Default for SerenaSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            auto_install: default_true(),
            command: None,
            version: default_serena_version(),
            context: default_serena_context(),
            modes: default_serena_modes(),
        }
    }
}

/// Language detection metadata. `binary` and `args` are not MCP launch commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetLsp {
    pub id: &'static str,
    pub language_key: &'static str,
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

pub fn default_serena_version() -> String {
    SERENA_VERSION.to_string()
}

pub fn default_serena_context() -> String {
    "codex".to_string()
}

pub fn default_serena_modes() -> Vec<String> {
    vec![
        "interactive".to_string(),
        "editing".to_string(),
        "planning".to_string(),
    ]
}

pub fn default_language_keys() -> Vec<String> {
    preset_lsp_servers()
        .iter()
        .filter(|preset| preset.default_checked)
        .map(|preset| preset.language_key.to_string())
        .collect()
}

pub fn default_config() -> CodeIntelConfig {
    CodeIntelConfig {
        enabled: false,
        lsp: LspSettings::default(),
        codegraph: CodegraphSettings::default(),
        serena: SerenaSettings::default(),
        migration_report: None,
    }
}

pub fn language_key_for_preset_id(id: &str) -> Option<&'static str> {
    preset_lsp_servers()
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.language_key)
}

pub fn preset_for_language_key(key: &str) -> Option<&'static PresetLsp> {
    preset_lsp_servers()
        .iter()
        .find(|preset| preset.language_key == key)
}

pub fn load_code_intel_config() -> CodeIntelConfig {
    load_code_intel_config_from(&code_intel_path())
}

pub fn load_code_intel_config_from(path: &Path) -> CodeIntelConfig {
    let Ok(bytes) = fs::read(path) else {
        return default_config();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return default_config();
    };
    if !value.is_object() {
        return default_config();
    }
    normalize_config(config_from_stored_value(&value))
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
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
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

pub fn normalize_config(mut config: CodeIntelConfig) -> CodeIntelConfig {
    config.lsp.max_concurrent = config.lsp.max_concurrent.clamp(1, 8);
    config.lsp.languages = normalize_language_keys(config.lsp.languages);
    config.codegraph.binary_path = sanitize_command_override(config.codegraph.binary_path);
    config.serena.command = sanitize_command_override(config.serena.command);
    config.serena.version = SERENA_VERSION.to_string();
    config.serena.context = normalize_context(&config.serena.context);
    config.serena.modes = normalize_modes(config.serena.modes);
    config
}

pub fn sanitize_command_override(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(is_shell_meta) {
        return None;
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Some(trimmed.to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if trimmed.starts_with('.') {
        return None;
    }
    Some(trimmed.to_string())
}

fn is_shell_meta(ch: char) -> bool {
    matches!(
        ch,
        '&' | '|'
            | ';'
            | '<'
            | '>'
            | '('
            | ')'
            | '$'
            | '`'
            | '\\'
            | '\''
            | '"'
            | '*'
            | '?'
            | '['
            | ']'
            | '{'
            | '}'
            | '!'
            | '#'
            | '~'
            | '\n'
            | '\r'
            | '\t'
            | '\0'
    )
}

pub fn normalize_context(raw: &str) -> String {
    let trimmed = raw.trim();
    if SERENA_CONTEXTS.contains(&trimmed) {
        trimmed.to_string()
    } else {
        default_serena_context()
    }
}

pub fn normalize_modes(modes: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for mode in modes {
        let trimmed = mode.trim();
        if SERENA_MODES.contains(&trimmed) && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    if out.is_empty() {
        default_serena_modes()
    } else {
        out
    }
}

fn normalize_language_keys(keys: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for key in keys {
        if preset_for_language_key(&key).is_some() && !out.iter().any(|existing| existing == &key) {
            out.push(key);
        }
    }
    out
}

fn config_from_stored_value(value: &Value) -> CodeIntelConfig {
    let mut cfg = default_config();
    let Some(root) = value.as_object() else {
        return cfg;
    };
    if let Some(enabled) = root.get("enabled").and_then(Value::as_bool) {
        cfg.enabled = enabled;
    }
    if let Some(codegraph) = root.get("codegraph").and_then(Value::as_object) {
        if let Some(enabled) = codegraph.get("enabled").and_then(Value::as_bool) {
            cfg.codegraph.enabled = enabled;
        }
        if let Some(auto_install) = codegraph.get("auto_install").and_then(Value::as_bool) {
            cfg.codegraph.auto_install = auto_install;
        }
        if codegraph.contains_key("binary_path") {
            cfg.codegraph.binary_path =
                sanitize_command_override(optional_string(codegraph.get("binary_path")));
        }
    }
    if let Some(lsp) = root.get("lsp").and_then(Value::as_object) {
        let legacy = !lsp.contains_key("languages")
            && (lsp.contains_key("auto_attach")
                || lsp.contains_key("checked")
                || lsp.contains_key("custom"));
        if legacy {
            if let Some(auto_attach) = lsp.get("auto_attach").and_then(Value::as_bool) {
                cfg.lsp.enabled = auto_attach;
            }
            if let Some(checked) = lsp.get("checked").and_then(Value::as_array) {
                cfg.lsp.languages = checked
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(language_key_for_preset_id)
                    .map(str::to_string)
                    .collect();
            }
        } else {
            if let Some(enabled) = lsp.get("enabled").and_then(Value::as_bool) {
                cfg.lsp.enabled = enabled;
            }
            if let Some(languages) = lsp.get("languages").and_then(Value::as_array) {
                cfg.lsp.languages = languages
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
            }
            if let Some(auto_install) = lsp.get("auto_install").and_then(Value::as_bool) {
                cfg.lsp.auto_install = auto_install;
            }
        }
        if let Some(max) = lsp.get("max_concurrent").and_then(json_u32) {
            cfg.lsp.max_concurrent = max;
        }
        if let Some(custom) = lsp.get("custom").and_then(Value::as_array) {
            cfg.migration_report = migration_report_from_custom(custom);
        }
    }
    if let Some(serena) = root.get("serena").and_then(Value::as_object) {
        if let Some(enabled) = serena.get("enabled").and_then(Value::as_bool) {
            cfg.serena.enabled = enabled;
        }
        if let Some(auto_install) = serena.get("auto_install").and_then(Value::as_bool) {
            cfg.serena.auto_install = auto_install;
        }
        if serena.contains_key("command") {
            cfg.serena.command = sanitize_command_override(optional_string(serena.get("command")));
        }
        if let Some(context) = serena.get("context").and_then(Value::as_str) {
            cfg.serena.context = context.to_string();
        }
        if let Some(modes) = serena.get("modes").and_then(Value::as_array) {
            cfg.serena.modes = modes
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
        }
    }
    cfg
}

fn migration_report_from_custom(custom: &[Value]) -> Option<String> {
    let mut parts = Vec::new();
    for item in custom {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or("unknown");
        let language = obj.get("language").and_then(Value::as_str).unwrap_or("");
        let command = obj.get("command").and_then(Value::as_str).unwrap_or("");
        parts.push(format!("{id} ({language}, command {command})"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "Ignored {} custom LSP server(s); they are not official MCP providers and were not started: {}.",
        parts.len(),
        parts.join("; ")
    ))
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

fn json_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .map(|number| u32::try_from(number).unwrap_or(u32::MAX))
        .or_else(|| value.as_i64().map(|number| number.max(0) as u32))
}

pub fn preset_lsp_servers() -> &'static [PresetLsp] {
    const PRESETS: &[PresetLsp] = &[
        PresetLsp {
            id: "rust-analyzer",
            language_key: "rust",
            language: "Rust",
            binary: "rust-analyzer",
            args: &[],
            manifests: &["Cargo.toml", "rust-toolchain.toml"],
            extensions: &[".rs"],
            default_checked: true,
        },
        PresetLsp {
            id: "gopls",
            language_key: "go",
            language: "Go",
            binary: "gopls",
            args: &[],
            manifests: &["go.mod", "go.work"],
            extensions: &[".go"],
            default_checked: true,
        },
        PresetLsp {
            id: "pyright",
            language_key: "python",
            language: "Python",
            binary: "pyright-langserver",
            args: &["--stdio"],
            manifests: &["pyproject.toml", "setup.py", "requirements.txt"],
            extensions: &[".py"],
            default_checked: true,
        },
        PresetLsp {
            id: "typescript",
            language_key: "typescript",
            language: "TypeScript / JavaScript",
            binary: "typescript-language-server",
            args: &["--stdio"],
            manifests: &["tsconfig.json", "jsconfig.json", "package.json"],
            extensions: &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts"],
            default_checked: true,
        },
        PresetLsp {
            id: "clangd",
            language_key: "cpp",
            language: "C / C++",
            binary: "clangd",
            args: &[],
            manifests: &["CMakeLists.txt", "compile_commands.json"],
            extensions: &[".c", ".h", ".cpp", ".hpp", ".cc", ".cxx"],
            default_checked: true,
        },
        PresetLsp {
            id: "lua-ls",
            language_key: "lua",
            language: "Lua",
            binary: "lua-language-server",
            args: &[],
            manifests: &[".luarc.json"],
            extensions: &[".lua"],
            default_checked: false,
        },
        PresetLsp {
            id: "bash-ls",
            language_key: "shell",
            language: "Shell",
            binary: "bash-language-server",
            args: &["start"],
            manifests: &[],
            extensions: &[".sh", ".bash"],
            default_checked: false,
        },
        PresetLsp {
            id: "yaml-ls",
            language_key: "yaml",
            language: "YAML",
            binary: "yaml-language-server",
            args: &["--stdio"],
            manifests: &[],
            extensions: &[".yaml", ".yml"],
            default_checked: false,
        },
        PresetLsp {
            id: "kotlin-ls",
            language_key: "kotlin",
            language: "Kotlin",
            binary: "kotlin-language-server",
            args: &[],
            manifests: &["build.gradle.kts"],
            extensions: &[".kt", ".kts"],
            default_checked: false,
        },
        PresetLsp {
            id: "zls",
            language_key: "zig",
            language: "Zig",
            binary: "zls",
            args: &[],
            manifests: &["build.zig"],
            extensions: &[".zig"],
            default_checked: false,
        },
        PresetLsp {
            id: "taplo",
            language_key: "toml",
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
    fn default_matches_three_provider_contract() {
        let cfg = default_config();
        assert!(!cfg.enabled);
        assert!(cfg.lsp.enabled);
        assert!(cfg.lsp.auto_install);
        assert_eq!(cfg.lsp.max_concurrent, 2);
        assert_eq!(
            cfg.lsp.languages,
            vec!["rust", "go", "python", "typescript", "cpp"]
        );
        assert!(cfg.codegraph.enabled);
        assert!(cfg.codegraph.auto_install);
        assert!(cfg.codegraph.binary_path.is_none());
        assert!(!cfg.serena.enabled);
        assert!(cfg.serena.auto_install);
        assert!(cfg.serena.command.is_none());
        assert_eq!(cfg.serena.version, "v1.7.0");
        assert_eq!(cfg.serena.context, "codex");
        assert_eq!(cfg.serena.modes, vec!["interactive", "editing", "planning"]);
        assert!(cfg.migration_report.is_none());
        let ids: Vec<_> = preset_lsp_servers()
            .iter()
            .map(|preset| preset.id)
            .collect();
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
    fn corrupt_file_returns_default_without_erasing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_code_intel_config_from(&path), default_config());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
    }

    #[test]
    fn legacy_file_migrates_languages_and_stashes_custom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(
            &path,
            r#"{
                "enabled": true,
                "lsp": {
                    "auto_attach": false,
                    "max_concurrent": 3,
                    "checked": ["rust-analyzer", "gopls", "unknown"],
                    "custom": [{
                        "id": "elixir-ls",
                        "language": "Elixir",
                        "command": "elixir-ls",
                        "args": ["--stdio"]
                    }]
                },
                "codegraph": {"enabled": true, "binary_path": "/opt/codegraph"}
            }"#,
        )
        .unwrap();
        let cfg = load_code_intel_config_from(&path);
        assert!(cfg.enabled);
        assert!(!cfg.lsp.enabled);
        assert_eq!(cfg.lsp.max_concurrent, 3);
        assert_eq!(cfg.lsp.languages, vec!["rust", "go"]);
        assert!(cfg.lsp.auto_install);
        assert_eq!(cfg.codegraph.binary_path.as_deref(), Some("/opt/codegraph"));
        assert!(!cfg.serena.enabled);
        assert_eq!(cfg.serena.version, "v1.7.0");
        let report = cfg.migration_report.expect("custom servers stashed");
        assert!(report.contains("elixir-ls"), "{report}");
        assert!(report.contains("Ignored"), "{report}");
        assert!(report.contains("not started"), "{report}");
        assert!(!report.contains("--stdio"), "{report}");
    }

    #[test]
    fn save_rewrites_only_the_new_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(
            &path,
            r#"{"enabled":true,"lsp":{"auto_attach":true,"checked":["clangd"],"custom":[{"id":"x","language":"X","command":"x"}]}}"#,
        )
        .unwrap();
        let loaded = load_code_intel_config_from(&path);
        assert!(loaded.migration_report.is_some());
        save_code_intel_config_to(&path, &loaded).unwrap();
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw.get("migration_report").is_none());
        assert!(raw["lsp"].get("languages").is_some());
        assert!(raw["lsp"].get("checked").is_none());
        assert!(raw["lsp"].get("custom").is_none());
        assert!(raw["lsp"].get("auto_attach").is_none());
        assert!(raw.get("serena").is_some());
        assert_eq!(raw["serena"]["version"], "v1.7.0");
        let again = load_code_intel_config_from(&path);
        assert!(again.migration_report.is_none());
        assert_eq!(again.lsp.languages, vec!["cpp"]);
    }

    #[test]
    fn round_trip_preserves_new_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("code-intel.json");
        let mut cfg = default_config();
        cfg.enabled = true;
        cfg.codegraph.binary_path = Some("/opt/my tools/codegraph".into());
        cfg.lsp.languages = vec!["rust".into(), "lua".into()];
        cfg.serena.enabled = true;
        cfg.serena.command = Some("serena".into());
        cfg.serena.context = "grok".into();
        cfg.serena.modes = vec!["editing".into(), "planning".into()];
        save_code_intel_config_to(&path, &cfg).unwrap();
        assert_eq!(load_code_intel_config_from(&path), cfg);
    }

    #[test]
    fn load_clamps_max_concurrent_and_rejects_bad_commands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("code-intel.json");
        std::fs::write(&path, r#"{"enabled":true,"lsp":{"max_concurrent":0}}"#).unwrap();
        assert_eq!(load_code_intel_config_from(&path).lsp.max_concurrent, 1);
        std::fs::write(&path, r#"{"lsp":{"max_concurrent":99}}"#).unwrap();
        assert_eq!(load_code_intel_config_from(&path).lsp.max_concurrent, 8);
        std::fs::write(
            &path,
            r#"{"codegraph":{"binary_path":"./codegraph"},"serena":{"command":"serena;rm","context":"nope","modes":["nope","editing"]}}"#,
        )
        .unwrap();
        let cfg = load_code_intel_config_from(&path);
        assert!(cfg.codegraph.binary_path.is_none());
        assert!(cfg.serena.command.is_none());
        assert_eq!(cfg.serena.context, "codex");
        assert_eq!(cfg.serena.modes, vec!["editing"]);
        std::fs::write(
            &path,
            r#"{"serena":{"modes":["nope"],"command":"/opt/my tools/serena"}}"#,
        )
        .unwrap();
        let cfg = load_code_intel_config_from(&path);
        assert_eq!(cfg.serena.modes, vec!["interactive", "editing", "planning"]);
        assert_eq!(cfg.serena.command.as_deref(), Some("/opt/my tools/serena"));
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
