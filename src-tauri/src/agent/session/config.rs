//! Composer session-config options for the in-process Codeg Agent.
//!
//! The host composer only renders what `session_config_options` advertises. A
//! model list without a thought-level selector is why the input showed just a
//! model id.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::acp::native_config::{EffectiveNativeConfig, WireProtocol};
use crate::acp::types::{
    SessionConfigKindInfo, SessionConfigOptionInfo, SessionConfigSelectInfo,
    SessionConfigSelectOptionInfo,
};
use crate::agent::mode::{available_mode_infos, parse_mode, MODE_CODE};

pub(crate) const PERMISSION_OPTION_ID: &str = "permission";
pub(crate) const MODE_OPTION_ID: &str = "mode";
pub(crate) const MODEL_OPTION_ID: &str = "model";
pub(crate) const THOUGHT_LEVEL_OPTION_ID: &str = "thought_level";
pub(crate) const DEFAULT_THOUGHT_LEVEL: &str = "off";
pub(crate) const DEFAULT_PERMISSION: &str = "ask";
pub(crate) const PERMISSION_ASK: &str = "ask";
pub(crate) const PERMISSION_ALLOW: &str = "allow";

/// Composer-facing thought levels. Values are OpenAI-compatible effort ids
/// (`none` is folded into `off` so the picker can actually disable reasoning).
const THOUGHT_LEVELS: &[(&str, &str, &str)] = &[
    ("off", "Off", "No extra reasoning"),
    ("minimal", "Minimal", "Lightest reasoning"),
    ("low", "Low", "Quick, fast responses"),
    ("medium", "Medium", "Balanced speed and quality"),
    ("high", "High", "Extensive reasoning for high quality"),
    ("xhigh", "Extra high", "Maximum reasoning for complex tasks"),
    ("max", "Max", "Highest reasoning depth"),
];

fn normalize_thought_level(value: &str) -> &str {
    match value.trim() {
        "none" => "off",
        other => other,
    }
}

pub(crate) fn canonical_thought_level(value: &str) -> Option<String> {
    let value = normalize_thought_level(value);
    THOUGHT_LEVELS
        .iter()
        .any(|(id, _, _)| *id == value)
        .then(|| value.to_string())
}

pub(crate) fn resolve_session_thought_level(preferred: &BTreeMap<String, String>) -> String {
    preferred
        .get(THOUGHT_LEVEL_OPTION_ID)
        .and_then(|s| canonical_thought_level(s))
        .unwrap_or_else(|| DEFAULT_THOUGHT_LEVEL.to_string())
}

const PERMISSION_LEVELS: &[(&str, &str, &str)] = &[
    (
        PERMISSION_ASK,
        "Ask",
        "Ask before write, bash, and other mutating tools",
    ),
    (PERMISSION_ALLOW, "Full access", "Run tools without asking"),
];

pub(crate) fn canonical_permission(value: &str) -> Option<String> {
    let value = value.trim();
    PERMISSION_LEVELS
        .iter()
        .any(|(id, _, _)| *id == value)
        .then(|| value.to_string())
}

pub(crate) fn resolve_session_permission(preferred: &BTreeMap<String, String>) -> String {
    preferred
        .get(PERMISSION_OPTION_ID)
        .and_then(|s| canonical_permission(s))
        .unwrap_or_else(|| DEFAULT_PERMISSION.to_string())
}

pub(crate) fn permission_is_full_access(value: &str) -> bool {
    canonical_permission(value).as_deref() == Some(PERMISSION_ALLOW)
}

pub(crate) fn resolve_session_mode(preferred: &BTreeMap<String, String>) -> Option<String> {
    preferred
        .get(MODE_OPTION_ID)
        .and_then(|s| parse_mode(s).map(str::to_string))
}

pub(crate) fn native_session_config_options(
    config: &EffectiveNativeConfig,
    model_id: &str,
    thought_level: &str,
    mode: &str,
    permission: &str,
) -> Vec<SessionConfigOptionInfo> {
    vec![
        native_session_permission_option(permission),
        native_session_mode_option(mode),
        native_session_model_option(config, model_id),
        native_session_thought_option(thought_level),
    ]
}

pub(crate) fn native_session_permission_option(current: &str) -> SessionConfigOptionInfo {
    let current = canonical_permission(current).unwrap_or_else(|| DEFAULT_PERMISSION.to_string());
    let options = PERMISSION_LEVELS
        .iter()
        .map(|(value, name, description)| SessionConfigSelectOptionInfo {
            value: (*value).to_string(),
            name: (*name).to_string(),
            description: Some((*description).to_string()),
        })
        .collect();
    SessionConfigOptionInfo {
        id: PERMISSION_OPTION_ID.into(),
        name: "Permission".into(),
        description: Some("Whether mutating tools wait for approval.".into()),
        category: Some("mode".into()),
        kind: SessionConfigKindInfo::Select(SessionConfigSelectInfo {
            current_value: current,
            options,
            groups: Vec::new(),
        }),
    }
}

pub(crate) fn native_session_mode_option(current: &str) -> SessionConfigOptionInfo {
    let current = parse_mode(current).unwrap_or(MODE_CODE);
    let options = available_mode_infos()
        .into_iter()
        .map(|mode| SessionConfigSelectOptionInfo {
            value: mode.id,
            name: mode.name,
            description: mode.description,
        })
        .collect();
    SessionConfigOptionInfo {
        id: MODE_OPTION_ID.into(),
        name: "Mode".into(),
        description: Some("Code implements; Plan is read-only planning.".into()),
        category: Some("mode".into()),
        kind: SessionConfigKindInfo::Select(SessionConfigSelectInfo {
            current_value: current.to_string(),
            options,
            groups: Vec::new(),
        }),
    }
}

/// Session model picker: only ids with an explicit window. Unknown ids are
/// rejected at SetConfigOption; windows are never assumed to be 128k.
pub(crate) fn native_session_model_option(
    config: &EffectiveNativeConfig,
    model_id: &str,
) -> SessionConfigOptionInfo {
    let options: Vec<SessionConfigSelectOptionInfo> = config
        .context_windows
        .iter()
        .map(|(id, window)| SessionConfigSelectOptionInfo {
            value: id.clone(),
            name: id.clone(),
            description: Some(format!("{window}-token window")),
        })
        .collect();
    SessionConfigOptionInfo {
        id: MODEL_OPTION_ID.into(),
        name: "Model".into(),
        description: Some(
            "Ids from CODEG_AGENT_CONTEXT_WINDOWS. Unknown models cannot start; \
             windows are not assumed to be 128k."
                .into(),
        ),
        category: Some("model".into()),
        kind: SessionConfigKindInfo::Select(SessionConfigSelectInfo {
            current_value: model_id.to_string(),
            options,
            groups: Vec::new(),
        }),
    }
}

pub(crate) fn native_session_thought_option(current: &str) -> SessionConfigOptionInfo {
    let current =
        canonical_thought_level(current).unwrap_or_else(|| DEFAULT_THOUGHT_LEVEL.to_string());
    let options = THOUGHT_LEVELS
        .iter()
        .map(|(value, name, description)| SessionConfigSelectOptionInfo {
            value: (*value).to_string(),
            name: (*name).to_string(),
            description: Some((*description).to_string()),
        })
        .collect();
    SessionConfigOptionInfo {
        id: THOUGHT_LEVEL_OPTION_ID.into(),
        name: "Thinking".into(),
        description: Some(
            "Reasoning effort sent to the bound provider. Off omits the parameter.".into(),
        ),
        category: Some("thought_level".into()),
        kind: SessionConfigKindInfo::Select(SessionConfigSelectInfo {
            current_value: current,
            options,
            groups: Vec::new(),
        }),
    }
}

/// Provider extra body for the current thought level. `off` sends nothing so
/// models that reject reasoning parameters keep working.
pub(crate) fn thought_level_additional_params(level: &str, wire: WireProtocol) -> Option<Value> {
    let level = canonical_thought_level(level)?;
    if level == "off" {
        return None;
    }
    match wire {
        WireProtocol::ChatCompletions => Some(json!({ "reasoning_effort": level })),
        WireProtocol::Responses => Some(json!({ "reasoning": { "effort": level } })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::native_config::CodegProtocol;

    fn sample_config() -> EffectiveNativeConfig {
        let mut windows = BTreeMap::new();
        windows.insert("gateway-model".into(), 128000);
        windows.insert("small".into(), 8192);
        EffectiveNativeConfig {
            api_base_url: "https://example.test/v1".into(),
            api_key: "sk".into(),
            model_id: "gateway-model".into(),
            context_windows: windows,
            max_output_tokens: 4096,
            system_prompt: None,
            compact_prompt: None,
            compact_soft_percent: 80,
            compact_recent_turns: 6,
            compact_model_id: None,
            compact_llm: false,
            max_turns: 40,
            protocol: CodegProtocol::ChatCompletions,
            resolved_protocol: None,
            context_inject: Default::default(),
        }
    }

    fn select_kind(
        option: &SessionConfigOptionInfo,
    ) -> &crate::acp::types::SessionConfigSelectInfo {
        match &option.kind {
            SessionConfigKindInfo::Select(select) => select,
            other => panic!("expected select, got {other:?}"),
        }
    }

    #[test]
    fn model_picker_lists_configured_windows_only() {
        let option = native_session_model_option(&sample_config(), "gateway-model");
        assert_eq!(option.id, "model");
        assert_eq!(option.category.as_deref(), Some("model"));
        let description = option.description.as_deref().expect("window copy");
        assert!(description.contains("CODEG_AGENT_CONTEXT_WINDOWS"));
        assert!(description.contains("128k"));
        let select = select_kind(&option);
        assert_eq!(select.current_value, "gateway-model");
        assert_eq!(select.options.len(), 2);
        let small = select
            .options
            .iter()
            .find(|o| o.value == "small")
            .expect("small");
        assert_eq!(small.description.as_deref(), Some("8192-token window"));
        let large = select
            .options
            .iter()
            .find(|o| o.value == "gateway-model")
            .expect("gateway");
        assert_eq!(large.description.as_deref(), Some("128000-token window"));
    }

    #[test]
    fn session_config_matches_codex_composer_order() {
        let options = native_session_config_options(
            &sample_config(),
            "gateway-model",
            "high",
            "plan",
            "allow",
        );
        let ids: Vec<_> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                PERMISSION_OPTION_ID,
                MODE_OPTION_ID,
                MODEL_OPTION_ID,
                THOUGHT_LEVEL_OPTION_ID
            ]
        );
        assert_eq!(options[0].name, "Permission");
        assert_eq!(select_kind(&options[0]).current_value, "allow");
        assert!(select_kind(&options[0])
            .options
            .iter()
            .any(|o| o.name == "Full access"));
        assert_eq!(options[1].name, "Mode");
        assert_eq!(select_kind(&options[1]).current_value, "plan");
        assert!(select_kind(&options[1])
            .options
            .iter()
            .any(|o| o.value == "code"));
        assert_eq!(options[2].id, MODEL_OPTION_ID);
        assert_eq!(options[3].name, "Thinking");
        assert_eq!(options[3].category.as_deref(), Some("thought_level"));
        let thought = select_kind(&options[3]);
        assert_eq!(thought.current_value, "high");
        assert!(thought.options.iter().any(|o| o.value == "off"));
        assert!(thought.options.iter().any(|o| o.value == "medium"));
        assert!(thought.options.iter().any(|o| o.value == "max"));
        let high = thought
            .options
            .iter()
            .find(|o| o.value == "high")
            .expect("high");
        assert_eq!(high.name, "High");
        assert!(high.description.as_deref().is_some());
    }

    #[test]
    fn unknown_permission_and_mode_clamp() {
        assert_eq!(
            select_kind(&native_session_permission_option("bogus")).current_value,
            "ask"
        );
        assert_eq!(
            select_kind(&native_session_mode_option("explore")).current_value,
            "code"
        );
        assert_eq!(resolve_session_permission(&BTreeMap::new()), "ask");
        let mut preferred = BTreeMap::new();
        preferred.insert(PERMISSION_OPTION_ID.into(), "allow".into());
        preferred.insert(MODE_OPTION_ID.into(), "plan".into());
        assert_eq!(resolve_session_permission(&preferred), "allow");
        assert_eq!(resolve_session_mode(&preferred).as_deref(), Some("plan"));
        assert!(permission_is_full_access("allow"));
        assert!(!permission_is_full_access("ask"));
    }

    #[test]
    fn unknown_thought_level_clamps_to_off() {
        let option = native_session_thought_option("bogus");
        assert_eq!(select_kind(&option).current_value, "off");
    }

    #[test]
    fn preferred_thought_level_is_applied() {
        let mut preferred = BTreeMap::new();
        preferred.insert(THOUGHT_LEVEL_OPTION_ID.into(), " high ".into());
        assert_eq!(resolve_session_thought_level(&preferred), "high");
        preferred.insert(THOUGHT_LEVEL_OPTION_ID.into(), "none".into());
        assert_eq!(resolve_session_thought_level(&preferred), "off");
        preferred.insert(THOUGHT_LEVEL_OPTION_ID.into(), "nope".into());
        assert_eq!(resolve_session_thought_level(&preferred), "off");
        assert_eq!(
            resolve_session_thought_level(&BTreeMap::new()),
            DEFAULT_THOUGHT_LEVEL
        );
    }

    #[test]
    fn off_sends_no_reasoning_params() {
        assert!(thought_level_additional_params("off", WireProtocol::ChatCompletions).is_none());
        assert!(thought_level_additional_params("none", WireProtocol::Responses).is_none());
        assert!(thought_level_additional_params("bogus", WireProtocol::ChatCompletions).is_none());
    }

    #[test]
    fn thought_level_params_follow_wire_protocol() {
        assert_eq!(
            thought_level_additional_params("high", WireProtocol::ChatCompletions),
            Some(json!({ "reasoning_effort": "high" }))
        );
        assert_eq!(
            thought_level_additional_params("xhigh", WireProtocol::Responses),
            Some(json!({ "reasoning": { "effort": "xhigh" } }))
        );
        assert_eq!(
            thought_level_additional_params("max", WireProtocol::Responses),
            Some(json!({ "reasoning": { "effort": "max" } }))
        );
    }
}
