//! Effective Codeg Agent model config: provider bind is authoritative.
//!
//! URL / key / model env keys are a projection of the bound provider, not a
//! second env-only auth path. Context windows and max-output live on agent env.

use std::collections::BTreeMap;

pub const API_BASE_URL_KEY: &str = "CODEG_AGENT_API_BASE_URL";
pub const API_KEY_KEY: &str = "CODEG_AGENT_API_KEY";
pub const MODEL_KEY: &str = "CODEG_AGENT_MODEL";
pub const CONTEXT_WINDOWS_KEY: &str = "CODEG_AGENT_CONTEXT_WINDOWS";
pub const MAX_OUTPUT_TOKENS_KEY: &str = "CODEG_AGENT_MAX_OUTPUT_TOKENS";
pub const SYSTEM_PROMPT_KEY: &str = "CODEG_AGENT_SYSTEM_PROMPT";
pub const COMPACT_PROMPT_KEY: &str = "CODEG_AGENT_COMPACT_PROMPT";
pub const COMPACT_SOFT_PERCENT_KEY: &str = "CODEG_AGENT_COMPACT_SOFT_PERCENT";
pub const COMPACT_RECENT_TURNS_KEY: &str = "CODEG_AGENT_COMPACT_RECENT_TURNS";
pub const COMPACT_MODEL_KEY: &str = "CODEG_AGENT_COMPACT_MODEL";
pub const PROTOCOL_KEY: &str = "CODEG_AGENT_PROTOCOL";
pub const RESOLVED_PROTOCOL_KEY: &str = "CODEG_AGENT_RESOLVED_PROTOCOL";
pub const MAX_TURNS_KEY: &str = "CODEG_AGENT_MAX_TURNS";
/// Spawn/preflight internal: set by [`overlay_bound_provider`] when a real
/// model-provider bind is projected this launch. Not a settings field.
/// `CODEG_AGENT_API_*` keys without this marker must not authenticate.
pub const PROVIDER_BOUND_KEY: &str = "CODEG_AGENT_PROVIDER_BOUND";
const PROVIDER_BOUND_VALUE: &str = "1";

pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;
pub const DEFAULT_COMPACT_SOFT_PERCENT: u8 = 80;
pub const DEFAULT_COMPACT_RECENT_TURNS: u32 = 6;
pub const DEFAULT_MAX_TURNS: u32 = 40;
/// Dummy Completions API key for loopback servers that ignore Authorization.
pub const LOCAL_API_KEY: &str = "local";
/// Reserved tokens that must remain after max-output is taken from the window.
pub const OUTPUT_SAFETY_MARGIN: u32 = 1024;

/// L2 LLM compact instruction when `CODEG_AGENT_COMPACT_PROMPT` is empty.
pub const DEFAULT_COMPACT_PROMPT: &str = "Summarize the evicted conversation turns. \
Output only the summary body. Preserve unfinished tool conclusions, user constraints, \
and file paths. Do not replay the original text of evicted turns.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundProvider {
    pub api_url: String,
    pub api_key: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNativeConfig {
    pub api_base_url: String,
    pub api_key: String,
    pub model_id: String,
    pub context_windows: BTreeMap<String, u32>,
    pub max_output_tokens: u32,
    /// Trimmed `CODEG_AGENT_SYSTEM_PROMPT`. `None` means the built-in preamble.
    pub system_prompt: Option<String>,
    /// Trimmed `CODEG_AGENT_COMPACT_PROMPT`. `None` means [`DEFAULT_COMPACT_PROMPT`].
    pub compact_prompt: Option<String>,
    /// Soft compact trigger as percent of the input budget. Default 80.
    pub compact_soft_percent: u8,
    /// Keep at least this many live turns before compacting. Default 6.
    pub compact_recent_turns: u32,
    /// Optional Completions id for L2 compact. `None` uses [`Self::model_id`].
    pub compact_model_id: Option<String>,
    /// Rig `max_turns` for one Prompt. Default 40.
    pub max_turns: u32,
    /// Provider request protocol. One session uses one resolved wire protocol.
    pub protocol: CodegProtocol,
    /// Last successful auto-detect result. Ignored unless `protocol` is Auto.
    pub resolved_protocol: Option<WireProtocol>,
}

/// How the bound provider wants the request layer to talk to the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegProtocol {
    Auto,
    ChatCompletions,
    Responses,
}

impl CodegProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
        }
    }

    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).unwrap_or("") {
            "auto" => Self::Auto,
            "responses" => Self::Responses,
            _ => Self::ChatCompletions,
        }
    }
}

/// Wire protocol actually used for every model call in one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireProtocol {
    ChatCompletions,
    Responses,
}

impl WireProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat_completions",
            Self::Responses => "responses",
        }
    }

    pub fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).unwrap_or("") {
            "chat_completions" => Some(Self::ChatCompletions),
            "responses" => Some(Self::Responses),
            _ => None,
        }
    }
}

impl EffectiveNativeConfig {
    /// Protocol this session will use. Auto without a probe result is Completions.
    pub fn wire_protocol(&self) -> WireProtocol {
        match self.protocol {
            CodegProtocol::ChatCompletions => WireProtocol::ChatCompletions,
            CodegProtocol::Responses => WireProtocol::Responses,
            CodegProtocol::Auto => self
                .resolved_protocol
                .unwrap_or(WireProtocol::ChatCompletions),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeConfigError {
    MissingProvider,
    EmptyApiKey,
    InvalidApiUrl,
    MissingModelId,
    InvalidContextWindows,
    MissingContextWindow { model_id: String },
    InvalidMaxOutputTokens,
    OutputExceedsWindow { max_output: u32, window: u32 },
}

impl NativeConfigError {
    pub fn check_id(&self) -> &'static str {
        match self {
            Self::MissingProvider | Self::EmptyApiKey | Self::InvalidApiUrl => "model_provider",
            Self::MissingModelId => "model_id",
            Self::InvalidContextWindows
            | Self::MissingContextWindow { .. }
            | Self::InvalidMaxOutputTokens
            | Self::OutputExceedsWindow { .. } => "context_window",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::MissingProvider => {
                "Bind a model provider — Codeg Agent does not read process-level OPENAI_* keys"
                    .into()
            }
            Self::EmptyApiKey => "Bound model provider has an empty API key".into(),
            Self::InvalidApiUrl => {
                "Bound model provider URL must be an http(s) endpoint".into()
            }
            Self::MissingModelId => "Bound model provider has no default model".into(),
            Self::InvalidContextWindows => {
                "CODEG_AGENT_CONTEXT_WINDOWS must be JSON of model id → positive integer window"
                    .into()
            }
            Self::MissingContextWindow { model_id } => format!(
                "No context window configured for model `{model_id}` in CODEG_AGENT_CONTEXT_WINDOWS"
            ),
            Self::InvalidMaxOutputTokens => {
                "CODEG_AGENT_MAX_OUTPUT_TOKENS must be a positive integer".into()
            }
            Self::OutputExceedsWindow {
                max_output,
                window,
            } => format!(
                "Max output {max_output} does not fit in window {window} after a {OUTPUT_SAFETY_MARGIN}-token safety margin"
            ),
        }
    }
}

/// Overlay bound-provider URL / key / model onto agent env, including empty
/// values (empty clears; never leave a leftover model or credential).
///
/// `model` is normalized to a Chat Completions id so a provider originally
/// created for Claude (JSON `main`) or Codex (catalog slug) can be bound to
/// Codeg Agent without writing the raw catalog JSON into `CODEG_AGENT_MODEL`.
///
/// Also writes [`PROVIDER_BOUND_KEY`] so spawn can tell this projection apart
/// from leftover `CODEG_AGENT_API_*` keys in env / env_json.
pub fn overlay_bound_provider(env: &mut BTreeMap<String, String>, provider: &BoundProvider) {
    overlay_env_value(env, API_BASE_URL_KEY, provider.api_url.trim());
    overlay_env_value(env, API_KEY_KEY, provider.api_key.trim());
    let model = completions_model_id(provider.model.as_deref()).unwrap_or_default();
    overlay_env_value(env, MODEL_KEY, &model);
    env.insert(
        PROVIDER_BOUND_KEY.to_string(),
        PROVIDER_BOUND_VALUE.to_string(),
    );
}

/// Project catalog protocol / windows while `provider.model` is still DB JSON.
/// Credential overlay must run after this and must not parse catalog again.
pub fn project_bound_provider_catalog(
    env: &mut BTreeMap<String, String>,
    provider: &BoundProvider,
) {
    match parse_codeg_catalog(provider.model.as_deref()) {
        Some(catalog) => {
            overlay_env_value(env, PROTOCOL_KEY, catalog.protocol.as_str());
            if catalog.windows.is_empty() {
                env.remove(CONTEXT_WINDOWS_KEY);
            } else if let Ok(raw) = serde_json::to_string(&catalog.windows) {
                env.insert(CONTEXT_WINDOWS_KEY.to_string(), raw);
            }
        }
        None => {
            env.remove(PROTOCOL_KEY);
            env.remove(RESOLVED_PROTOCOL_KEY);
        }
    }
}

struct ParsedCodegCatalog {
    protocol: CodegProtocol,
    windows: BTreeMap<String, u32>,
}

fn parse_codeg_catalog(raw: Option<&str>) -> Option<ParsedCodegCatalog> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = value.as_object()?;
    if obj.get("kind").and_then(serde_json::Value::as_str) != Some("codeg_agent_catalog") {
        return None;
    }
    let models = obj.get("models")?.as_array()?;
    let mut windows = BTreeMap::new();
    for item in models {
        let id = if let Some(id) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            id.to_string()
        } else {
            let row = item.as_object()?;
            row.get("id")
                .or_else(|| row.get("slug"))
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string()
        };
        let window = item
            .as_object()
            .and_then(|row| {
                row.get("context_window")
                    .or_else(|| row.get("contextWindow"))
            })
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0)
            .map(|n| n as u32)
            .unwrap_or(128_000);
        windows.insert(id, window);
    }
    if windows.is_empty() {
        return None;
    }
    Some(ParsedCodegCatalog {
        protocol: CodegProtocol::parse(obj.get("protocol").and_then(serde_json::Value::as_str)),
        windows,
    })
}

/// True only when [`overlay_bound_provider`] projected a bind this launch.
pub fn env_has_provider_bind(env: &BTreeMap<String, String>) -> bool {
    env.get(PROVIDER_BOUND_KEY).map(String::as_str) == Some(PROVIDER_BOUND_VALUE)
}

/// Extract a Chat Completions model id from a model-provider `model` column.
///
/// Plain slugs pass through. Claude JSON uses `main` (then `customOption`).
/// Codex JSON uses `default`, then the first `customs`/`models` slug.
/// Unrecognized JSON is rejected rather than used as a model id.
pub fn completions_model_id(raw: Option<&str>) -> Option<String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty())?;
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) {
        return model_id_from_json(&value);
    }
    Some(raw.to_string())
}

fn model_id_from_json(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        serde_json::Value::Object(obj) => {
            for key in ["main", "customOption", "default"] {
                if let Some(id) = obj
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return Some(id.to_string());
                }
            }
            for list_key in ["customs", "models"] {
                let Some(arr) = obj.get(list_key).and_then(serde_json::Value::as_array) else {
                    continue;
                };
                for item in arr {
                    if let Some(slug) = item
                        .get("slug")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        return Some(slug.to_string());
                    }
                    if let Some(id) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        return Some(id.to_string());
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn overlay_env_value(env: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if value.is_empty() {
        env.remove(key);
    } else {
        env.insert(key.to_string(), value.to_string());
    }
}

pub fn resolve_codeg_agent_config(
    agent_env: &BTreeMap<String, String>,
    provider: Option<&BoundProvider>,
) -> Result<EffectiveNativeConfig, NativeConfigError> {
    let Some(provider) = provider else {
        return Err(NativeConfigError::MissingProvider);
    };

    let mut env = agent_env.clone();
    overlay_bound_provider(&mut env, provider);

    let api_base_url = env
        .get(API_BASE_URL_KEY)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(NativeConfigError::InvalidApiUrl)?;
    if !is_http_url(&api_base_url) {
        return Err(NativeConfigError::InvalidApiUrl);
    }

    let api_key = env
        .get(API_KEY_KEY)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| is_loopback_http_url(&api_base_url).then(|| LOCAL_API_KEY.to_string()))
        .ok_or(NativeConfigError::EmptyApiKey)?;

    let model_id = env
        .get(MODEL_KEY)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or(NativeConfigError::MissingModelId)?;

    let context_windows = parse_context_windows(env.get(CONTEXT_WINDOWS_KEY).map(String::as_str))?;
    let window = context_windows.get(&model_id).copied().ok_or_else(|| {
        NativeConfigError::MissingContextWindow {
            model_id: model_id.clone(),
        }
    })?;

    let max_output_tokens = parse_max_output(env.get(MAX_OUTPUT_TOKENS_KEY).map(String::as_str))?;
    let usable = window.saturating_sub(OUTPUT_SAFETY_MARGIN);
    if max_output_tokens == 0 || max_output_tokens > usable {
        return Err(NativeConfigError::OutputExceedsWindow {
            max_output: max_output_tokens,
            window,
        });
    }

    Ok(EffectiveNativeConfig {
        api_base_url,
        api_key,
        model_id,
        context_windows,
        max_output_tokens,
        system_prompt: trimmed_optional(env.get(SYSTEM_PROMPT_KEY).map(String::as_str)),
        compact_prompt: trimmed_optional(env.get(COMPACT_PROMPT_KEY).map(String::as_str)),
        compact_soft_percent: parse_percent(
            env.get(COMPACT_SOFT_PERCENT_KEY).map(String::as_str),
            DEFAULT_COMPACT_SOFT_PERCENT,
        ),
        compact_recent_turns: parse_positive_u32(
            env.get(COMPACT_RECENT_TURNS_KEY).map(String::as_str),
            DEFAULT_COMPACT_RECENT_TURNS,
        ),
        compact_model_id: trimmed_optional(env.get(COMPACT_MODEL_KEY).map(String::as_str)),
        max_turns: parse_positive_u32(
            env.get(MAX_TURNS_KEY).map(String::as_str),
            DEFAULT_MAX_TURNS,
        ),
        protocol: CodegProtocol::parse(env.get(PROTOCOL_KEY).map(String::as_str)),
        resolved_protocol: WireProtocol::parse(env.get(RESOLVED_PROTOCOL_KEY).map(String::as_str)),
    })
}

/// Empty / whitespace-only env values are absent so spawn uses the builtin default.
pub fn trimmed_optional(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn effective_compact_prompt(configured: Option<&str>) -> &str {
    configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_COMPACT_PROMPT)
}

fn is_http_url(raw: &str) -> bool {
    let raw = raw.trim();
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"));
    match rest {
        Some(host_and_path) => {
            let host = host_and_path.split('/').next().unwrap_or("");
            !host.is_empty() && !raw.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

fn http_host(raw: &str) -> Option<&str> {
    let rest = raw
        .trim()
        .strip_prefix("https://")
        .or_else(|| raw.trim().strip_prefix("http://"))?;
    let hostport = rest.split('/').next().unwrap_or("");
    let host = hostport
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(hostport);
    let host = if host.starts_with('[') {
        host.trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or(host)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    Some(host)
}

/// Local OpenAI-compatible servers (Ollama, LM Studio) typically ignore auth.
pub fn is_loopback_http_url(raw: &str) -> bool {
    matches!(
        http_host(raw).map(|h| h.to_ascii_lowercase()),
        Some(h) if h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "0.0.0.0"
    )
}

fn parse_percent(raw: Option<&str>, default: u8) -> u8 {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return default;
    };
    raw.parse::<u8>()
        .ok()
        .filter(|v| (1..=100).contains(v))
        .unwrap_or(default)
}

fn parse_positive_u32(raw: Option<&str>, default: u32) -> u32 {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return default;
    };
    raw.parse::<u32>()
        .ok()
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn parse_context_windows(raw: Option<&str>) -> Result<BTreeMap<String, u32>, NativeConfigError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(BTreeMap::new());
    };
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| NativeConfigError::InvalidContextWindows)?;
    let obj = parsed
        .as_object()
        .ok_or(NativeConfigError::InvalidContextWindows)?;
    let mut out = BTreeMap::new();
    for (model, value) in obj {
        let model = model.trim();
        if model.is_empty() {
            return Err(NativeConfigError::InvalidContextWindows);
        }
        let window = match value {
            serde_json::Value::Number(n) => n
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .filter(|v| *v > 0),
            serde_json::Value::String(s) => s.trim().parse::<u32>().ok().filter(|v| *v > 0),
            _ => None,
        }
        .ok_or(NativeConfigError::InvalidContextWindows)?;
        out.insert(model.to_string(), window);
    }
    Ok(out)
}

fn parse_max_output(raw: Option<&str>) -> Result<u32, NativeConfigError> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_MAX_OUTPUT_TOKENS);
    };
    raw.parse::<u32>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or(NativeConfigError::InvalidMaxOutputTokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(url: &str, key: &str, model: &str) -> BoundProvider {
        BoundProvider {
            api_url: url.into(),
            api_key: key.into(),
            model: Some(model.into()),
        }
    }

    fn env_with_windows(model: &str, window: u32) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert(
            CONTEXT_WINDOWS_KEY.into(),
            format!(r#"{{"{model}": {window}}}"#),
        );
        env
    }

    #[test]
    fn missing_provider_fails_without_reading_env_creds() {
        let mut env = env_with_windows("gpt-4.1", 128000);
        env.insert(API_KEY_KEY.into(), "sk-from-env".into());
        env.insert(API_BASE_URL_KEY.into(), "https://api.example.com/v1".into());
        env.insert(MODEL_KEY.into(), "gpt-4.1".into());
        assert!(
            !env_has_provider_bind(&env),
            "leftover CODEG_AGENT_API_* keys are not a bind"
        );
        assert_eq!(
            resolve_codeg_agent_config(&env, None),
            Err(NativeConfigError::MissingProvider)
        );
    }

    #[test]
    fn overlay_sets_internal_bind_marker_and_resolves() {
        let mut env = env_with_windows("gpt-4.1", 128000);
        env.insert(API_KEY_KEY.into(), "sk-leftover".into());
        let p = provider("https://gw.example/v1", "sk-bound", "gpt-4.1");
        overlay_bound_provider(&mut env, &p);
        assert!(env_has_provider_bind(&env));
        assert_eq!(env.get(PROVIDER_BOUND_KEY).map(String::as_str), Some("1"));
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.api_key, "sk-bound");
        assert_eq!(cfg.model_id, "gpt-4.1");
        assert_eq!(cfg.api_base_url, "https://gw.example/v1");
    }

    #[test]
    fn provider_overlay_clears_stale_model() {
        let mut env = env_with_windows("fresh", 128000);
        env.insert(MODEL_KEY.into(), "stale".into());
        env.insert(API_KEY_KEY.into(), "old-key".into());
        let p = provider("https://gw.example/v1", "sk-new", "fresh");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.model_id, "fresh");
        assert_eq!(cfg.api_key, "sk-new");
        assert_eq!(cfg.max_output_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn empty_provider_model_does_not_keep_old_env_model() {
        let mut env = env_with_windows("stale", 128000);
        env.insert(MODEL_KEY.into(), "stale".into());
        let p = BoundProvider {
            api_url: "https://gw.example/v1".into(),
            api_key: "sk-new".into(),
            model: None,
        };
        assert_eq!(
            resolve_codeg_agent_config(&env, Some(&p)),
            Err(NativeConfigError::MissingModelId)
        );
    }

    #[test]
    fn unknown_window_is_not_defaulted() {
        let env = env_with_windows("other", 128000);
        let p = provider("https://gw.example/v1", "sk", "gateway-model");
        assert!(matches!(
            resolve_codeg_agent_config(&env, Some(&p)),
            Err(NativeConfigError::MissingContextWindow { model_id }) if model_id == "gateway-model"
        ));
    }

    #[test]
    fn rejects_non_http_url() {
        let env = env_with_windows("m", 128000);
        let p = provider("not-a-url", "sk", "m");
        assert_eq!(
            resolve_codeg_agent_config(&env, Some(&p)),
            Err(NativeConfigError::InvalidApiUrl)
        );
    }

    #[test]
    fn output_must_fit_after_safety_margin() {
        let mut env = env_with_windows("m", 4096);
        env.insert(MAX_OUTPUT_TOKENS_KEY.into(), "4096".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        assert!(matches!(
            resolve_codeg_agent_config(&env, Some(&p)),
            Err(NativeConfigError::OutputExceedsWindow { .. })
        ));
    }

    #[test]
    fn valid_window_and_default_output() {
        let env = env_with_windows("m", 128000);
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.context_windows.get("m"), Some(&128000));
        assert_eq!(cfg.max_output_tokens, 4096);
        assert_eq!(cfg.system_prompt, None);
        assert_eq!(cfg.compact_prompt, None);
        assert_eq!(
            effective_compact_prompt(cfg.compact_prompt.as_deref()),
            DEFAULT_COMPACT_PROMPT
        );
    }

    #[test]
    fn empty_prompts_trim_to_builtin_default() {
        let mut env = env_with_windows("m", 128000);
        env.insert(SYSTEM_PROMPT_KEY.into(), "   ".into());
        env.insert(COMPACT_PROMPT_KEY.into(), "\n".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.system_prompt, None);
        assert_eq!(cfg.compact_prompt, None);
    }

    #[test]
    fn configured_prompts_are_snapshotted() {
        let mut env = env_with_windows("m", 128000);
        env.insert(SYSTEM_PROMPT_KEY.into(), " Be terse. ".into());
        env.insert(COMPACT_PROMPT_KEY.into(), " Keep paths; no replay. ".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.system_prompt.as_deref(), Some("Be terse."));
        assert_eq!(
            cfg.compact_prompt.as_deref(),
            Some("Keep paths; no replay.")
        );
        assert_eq!(
            effective_compact_prompt(cfg.compact_prompt.as_deref()),
            "Keep paths; no replay."
        );
    }

    #[test]
    fn claude_json_provider_uses_main_as_completions_model() {
        let env = env_with_windows("claude-sonnet-5", 128000);
        let p = provider(
            "https://gw.example/v1",
            "sk",
            r#"{"main":"claude-sonnet-5","reasoning":"claude-opus-4"}"#,
        );
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.model_id, "claude-sonnet-5");
    }

    #[test]
    fn codex_catalog_provider_uses_default_then_first_slug() {
        assert_eq!(
            completions_model_id(Some(
                r#"{"customs":[{"slug":"gpt-4.1","base":"gpt-4.1"}],"default":"gpt-4.1"}"#
            )),
            Some("gpt-4.1".into())
        );
        assert_eq!(
            completions_model_id(Some(r#"{"models":[{"slug":"o4-mini"}]}"#)),
            Some("o4-mini".into())
        );
        assert_eq!(
            completions_model_id(Some("gateway-model")),
            Some("gateway-model".into())
        );
        assert_eq!(completions_model_id(Some(r#"{"unrelated":true}"#)), None);
        assert_eq!(
            completions_model_id(Some(
                r#"{"kind":"codeg_agent_catalog","default":"b","models":[{"id":"a"},{"id":"b"}]}"#
            )),
            Some("b".into())
        );
    }

    #[test]
    fn catalog_bind_projects_protocol_and_windows() {
        let mut env = BTreeMap::new();
        let p = BoundProvider {
            api_url: "https://gw.example/v1".into(),
            api_key: "sk".into(),
            model: Some(
                r#"{"kind":"codeg_agent_catalog","protocol":"responses","default":"b","models":[{"id":"a","context_window":32000},{"id":"b","context_window":64000}]}"#
                    .into(),
            ),
        };
        project_bound_provider_catalog(&mut env, &p);
        overlay_bound_provider(&mut env, &p);
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.protocol, CodegProtocol::Responses);
        assert_eq!(cfg.wire_protocol(), WireProtocol::Responses);
        assert_eq!(cfg.model_id, "b");
        assert_eq!(cfg.context_windows.get("a"), Some(&32000));
        assert_eq!(cfg.context_windows.get("b"), Some(&64000));
    }

    #[test]
    fn compact_model_id_is_optional_override() {
        let mut env = env_with_windows("m", 128000);
        env.insert(COMPACT_MODEL_KEY.into(), " summarizer ".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.compact_model_id.as_deref(), Some("summarizer"));
        assert_eq!(cfg.model_id, "m");
    }

    #[test]
    fn loopback_url_allows_empty_api_key() {
        let env = env_with_windows("llama3", 8192);
        let p = BoundProvider {
            api_url: "http://127.0.0.1:11434/v1".into(),
            api_key: String::new(),
            model: Some("llama3".into()),
        };
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("local ollama");
        assert_eq!(cfg.api_key, LOCAL_API_KEY);
        assert_eq!(cfg.model_id, "llama3");
    }

    #[test]
    fn remote_url_still_requires_api_key() {
        let env = env_with_windows("m", 128000);
        let p = BoundProvider {
            api_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: Some("m".into()),
        };
        assert_eq!(
            resolve_codeg_agent_config(&env, Some(&p)),
            Err(NativeConfigError::EmptyApiKey)
        );
    }

    #[test]
    fn compact_and_runtime_env_keys_parse() {
        let mut env = env_with_windows("m", 128000);
        env.insert(COMPACT_SOFT_PERCENT_KEY.into(), "70".into());
        env.insert(COMPACT_RECENT_TURNS_KEY.into(), "8".into());
        env.insert(MAX_TURNS_KEY.into(), "24".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.compact_soft_percent, 70);
        assert_eq!(cfg.compact_recent_turns, 8);
        assert_eq!(cfg.max_turns, 24);
    }

    #[test]
    fn invalid_compact_and_runtime_env_fall_back_to_defaults() {
        let mut env = env_with_windows("m", 128000);
        env.insert(COMPACT_SOFT_PERCENT_KEY.into(), "0".into());
        env.insert(COMPACT_RECENT_TURNS_KEY.into(), "nope".into());
        env.insert(MAX_TURNS_KEY.into(), "-1".into());
        let p = provider("https://gw.example/v1", "sk", "m");
        let cfg = resolve_codeg_agent_config(&env, Some(&p)).expect("valid");
        assert_eq!(cfg.compact_soft_percent, DEFAULT_COMPACT_SOFT_PERCENT);
        assert_eq!(cfg.compact_recent_turns, DEFAULT_COMPACT_RECENT_TURNS);
        assert_eq!(cfg.max_turns, DEFAULT_MAX_TURNS);
    }
}
