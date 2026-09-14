//! Session-scoped LLM client. One session uses one wire protocol.

use std::time::Duration;

use rig::providers::openai::{Client as ResponsesClient, CompletionsClient};
use serde_json::json;

use crate::acp::native_config::{CodegProtocol, EffectiveNativeConfig, WireProtocol};

use super::CompletionsClientError;

/// Completions or Responses client chosen once at session start.
#[derive(Clone)]
pub enum CodegLlmClient {
    Completions(CompletionsClient),
    Responses(ResponsesClient),
}

impl CodegLlmClient {
    pub fn build(
        api_key: impl Into<String>,
        api_base_url: impl AsRef<str>,
        wire: WireProtocol,
    ) -> Result<Self, CompletionsClientError> {
        let api_key = api_key.into();
        let base = api_base_url.as_ref();
        match wire {
            WireProtocol::ChatCompletions => CompletionsClient::builder()
                .api_key(api_key)
                .base_url(base)
                .build()
                .map(Self::Completions)
                .map_err(|err| CompletionsClientError(err.to_string())),
            WireProtocol::Responses => ResponsesClient::builder()
                .api_key(api_key)
                .base_url(base)
                .build()
                .map(Self::Responses)
                .map_err(|err| CompletionsClientError(err.to_string())),
        }
    }
}

/// Resolve the single wire protocol this session will use.
///
/// Auto probes Completions first. A 404 means a Responses-only gateway.
/// Any other outcome (2xx, 4xx auth, network) stays Completions so spawn
/// does not hang waiting for a Responses-only guess.
pub async fn resolve_session_wire_protocol(config: &EffectiveNativeConfig) -> WireProtocol {
    match config.protocol {
        CodegProtocol::ChatCompletions => WireProtocol::ChatCompletions,
        CodegProtocol::Responses => WireProtocol::Responses,
        CodegProtocol::Auto => {
            if let Some(resolved) = config.resolved_protocol {
                return resolved;
            }
            probe_completions_then_responses(&config.api_base_url, &config.api_key).await
        }
    }
}

async fn probe_completions_then_responses(base_url: &str, api_key: &str) -> WireProtocol {
    let url = format!("{}/chat/completions", base_url.trim().trim_end_matches('/'));
    let request = json!({
        "model": "codeg-protocol-probe",
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });
    match reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .json(&request)
        .timeout(Duration::from_secs(8))
        .send()
        .await
    {
        Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => {
            WireProtocol::Responses
        }
        _ => WireProtocol::ChatCompletions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::native_config::CodegProtocol;

    #[test]
    fn explicit_protocols_do_not_need_a_probe() {
        let completions = EffectiveNativeConfig {
            api_base_url: "https://example.test/v1".into(),
            api_key: "sk".into(),
            model_id: "m".into(),
            context_windows: Default::default(),
            max_output_tokens: 4096,
            system_prompt: None,
            compact_prompt: None,
            compact_soft_percent: 80,
            compact_recent_turns: 6,
            compact_model_id: None,
            max_turns: 40,
            protocol: CodegProtocol::ChatCompletions,
            resolved_protocol: None,
        };
        assert_eq!(completions.wire_protocol(), WireProtocol::ChatCompletions);
        let mut responses = completions.clone();
        responses.protocol = CodegProtocol::Responses;
        assert_eq!(responses.wire_protocol(), WireProtocol::Responses);
        let mut auto = completions;
        auto.protocol = CodegProtocol::Auto;
        auto.resolved_protocol = Some(WireProtocol::Responses);
        assert_eq!(auto.wire_protocol(), WireProtocol::Responses);
    }
}
