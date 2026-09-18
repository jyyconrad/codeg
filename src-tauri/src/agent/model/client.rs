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
/// Concrete catalog values are used as-is. `auto` (legacy / unbound) still
/// probes Completions first, then Responses when Completions is missing or
/// ambiguous (404/405, or 5xx/400 that Responses accepts).
pub async fn resolve_session_wire_protocol(config: &EffectiveNativeConfig) -> WireProtocol {
    match config.protocol {
        CodegProtocol::ChatCompletions => WireProtocol::ChatCompletions,
        CodegProtocol::Responses => WireProtocol::Responses,
        CodegProtocol::Auto => {
            if let Some(resolved) = config.resolved_protocol {
                return resolved;
            }
            probe_wire_protocol(&config.api_base_url, &config.api_key, &config.model_id).await
        }
    }
}

#[derive(Clone, Copy)]
enum ProbeClass {
    Present,
    Absent,
    Uncertain,
}

/// Completions-first protocol probe. Bind locks this result onto the catalog.
pub async fn probe_wire_protocol(base_url: &str, api_key: &str, model_id: &str) -> WireProtocol {
    let base = base_url.trim().trim_end_matches('/');
    let model = model_id.trim();
    let model = if model.is_empty() { "probe" } else { model };
    let client = reqwest::Client::new();
    match classify_probe(probe_chat_completions(&client, base, api_key, model).await) {
        ProbeClass::Present => WireProtocol::ChatCompletions,
        ProbeClass::Absent => WireProtocol::Responses,
        ProbeClass::Uncertain => {
            match classify_probe(probe_responses(&client, base, api_key, model).await) {
                ProbeClass::Present => WireProtocol::Responses,
                ProbeClass::Absent | ProbeClass::Uncertain => WireProtocol::ChatCompletions,
            }
        }
    }
}

fn classify_probe(status: Option<reqwest::StatusCode>) -> ProbeClass {
    match status {
        Some(status)
            if status.is_success()
                || status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
        {
            ProbeClass::Present
        }
        Some(status)
            if status == reqwest::StatusCode::NOT_FOUND
                || status == reqwest::StatusCode::METHOD_NOT_ALLOWED =>
        {
            ProbeClass::Absent
        }
        Some(_) | None => ProbeClass::Uncertain,
    }
}

async fn probe_chat_completions(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    model: &str,
) -> Option<reqwest::StatusCode> {
    let url = format!("{base}/chat/completions");
    let request = json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });
    probe_post(client, &url, api_key, request).await
}

async fn probe_responses(
    client: &reqwest::Client,
    base: &str,
    api_key: &str,
    model: &str,
) -> Option<reqwest::StatusCode> {
    let url = format!("{base}/responses");
    let request = json!({
        "model": model,
        "input": "ping",
        "max_output_tokens": 1,
    });
    probe_post(client, &url, api_key, request).await
}

async fn probe_post(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    request: serde_json::Value,
) -> Option<reqwest::StatusCode> {
    client
        .post(url)
        .bearer_auth(api_key)
        .json(&request)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()
        .map(|response| response.status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::native_config::CodegProtocol;
    use axum::extract::Json;
    use axum::http::{header, StatusCode, Uri};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::Router;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct ProbeCapture {
        paths: Arc<Mutex<Vec<String>>>,
        bodies: Arc<Mutex<Vec<Value>>>,
        completions: StatusCode,
        responses: StatusCode,
    }

    async fn spawn_probe_gateway(
        completions: StatusCode,
        responses: StatusCode,
    ) -> (String, ProbeCapture) {
        let capture = ProbeCapture {
            paths: Arc::new(Mutex::new(Vec::new())),
            bodies: Arc::new(Mutex::new(Vec::new())),
            completions,
            responses,
        };
        let state = capture.clone();
        let app = Router::new().fallback(post(move |uri: Uri, Json(body): Json<Value>| {
            let state = state.clone();
            async move {
                state
                    .paths
                    .lock()
                    .expect("paths")
                    .push(uri.path().to_string());
                state.bodies.lock().expect("bodies").push(body);
                let status = if uri.path().ends_with("/responses") {
                    state.responses
                } else {
                    state.completions
                };
                (
                    status,
                    [(header::CONTENT_TYPE, "application/json")],
                    "{\"ok\":true}",
                )
                    .into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind probe gateway");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/v1"), capture)
    }

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
            compact_llm: false,
            max_turns: 40,
            protocol: CodegProtocol::ChatCompletions,
            resolved_protocol: None,
            context_inject: Default::default(),
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

    #[tokio::test]
    async fn completions_ok_locks_chat_completions() {
        let (base, capture) = spawn_probe_gateway(StatusCode::OK, StatusCode::OK).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::ChatCompletions);
        let paths = capture.paths.lock().expect("paths").clone();
        assert!(
            paths.iter().any(|p| p.ends_with("/chat/completions")),
            "{paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.ends_with("/responses")),
            "must not probe Responses after Completions succeeds: {paths:?}"
        );
        let body = &capture.bodies.lock().expect("bodies")[0];
        assert_eq!(
            body.get("model").and_then(Value::as_str),
            Some("gateway-model")
        );
    }

    #[tokio::test]
    async fn completions_404_locks_responses() {
        let (base, _) = spawn_probe_gateway(StatusCode::NOT_FOUND, StatusCode::OK).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::Responses);
    }

    #[tokio::test]
    async fn completions_500_then_responses_ok_locks_responses() {
        let (base, capture) =
            spawn_probe_gateway(StatusCode::INTERNAL_SERVER_ERROR, StatusCode::OK).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::Responses);
        let paths = capture.paths.lock().expect("paths").clone();
        assert!(
            paths.iter().any(|p| p.ends_with("/chat/completions")),
            "{paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("/responses")),
            "Completions 500 must try Responses: {paths:?}"
        );
        let bodies = capture.bodies.lock().expect("bodies").clone();
        assert!(
            bodies
                .iter()
                .all(|body| body.get("model").and_then(Value::as_str) == Some("gateway-model")),
            "{bodies:?}"
        );
        assert!(
            !bodies
                .iter()
                .any(|body| body.get("model").and_then(Value::as_str)
                    == Some("codeg-protocol-probe")),
            "probe must use the bound model id, not a dummy: {bodies:?}"
        );
    }

    #[tokio::test]
    async fn completions_500_then_responses_404_stays_completions() {
        let (base, _) =
            spawn_probe_gateway(StatusCode::INTERNAL_SERVER_ERROR, StatusCode::NOT_FOUND).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::ChatCompletions);
    }

    #[tokio::test]
    async fn completions_429_locks_chat_completions() {
        let (base, capture) =
            spawn_probe_gateway(StatusCode::TOO_MANY_REQUESTS, StatusCode::OK).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::ChatCompletions);
        let paths = capture.paths.lock().expect("paths").clone();
        assert!(
            !paths.iter().any(|p| p.ends_with("/responses")),
            "rate-limit on Completions means the path exists: {paths:?}"
        );
    }

    #[tokio::test]
    async fn completions_401_locks_chat_completions() {
        let (base, capture) = spawn_probe_gateway(StatusCode::UNAUTHORIZED, StatusCode::OK).await;
        let wire = probe_wire_protocol(&base, "sk", "gateway-model").await;
        assert_eq!(wire, WireProtocol::ChatCompletions);
        let paths = capture.paths.lock().expect("paths").clone();
        assert!(
            !paths.iter().any(|p| p.ends_with("/responses")),
            "auth on Completions means the path exists: {paths:?}"
        );
    }
}
