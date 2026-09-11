mod preamble;
mod turn;

use rig::providers::openai::CompletionsClient;

pub use preamble::session_preamble;
pub use turn::{run_native_turn, NativeTurnOutcome, NativeTurnRequest, NativeTurnTools};

/// Total model-call budget for one Prompt (initial call plus retries).
pub const DEFAULT_MAX_TURNS: usize = 40;
/// Serial tool execution so permission cards stay one-at-a-time.
pub const DEFAULT_TOOL_CONCURRENCY: usize = 1;
/// Invalid tool-call retries before the run fails closed. v1 Retry only.
pub const DEFAULT_INVALID_TOOL_CALL_RETRIES: usize = 2;

#[derive(Debug, thiserror::Error)]
#[error("completions client: {0}")]
pub struct CompletionsClientError(String);

/// OpenAI Chat Completions client. Do not use the default Responses `openai::Client`.
///
/// `api_base_url` is the API root including `/v1`; the provider appends
/// `/chat/completions`.
pub fn completions_client(
    api_key: impl Into<String>,
    api_base_url: impl AsRef<str>,
) -> Result<CompletionsClient, CompletionsClientError> {
    CompletionsClient::builder()
        .api_key(api_key.into())
        .base_url(api_base_url.as_ref())
        .build()
        .map_err(|err| CompletionsClientError(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_base_url_builds_a_completions_client() {
        completions_client("sk-test", "https://api.example.test/v1")
            .expect("TLS-capable CompletionsClient");
    }

    #[test]
    fn lockfile_pins_rig_agent_core_and_rmcp() {
        let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"));
        assert_eq!(
            package_versions(lock, "rig"),
            vec!["0.42.0".to_string()],
            "rig facade must stay 0.42.0"
        );
        assert_eq!(
            package_versions(lock, "rig-core"),
            vec!["0.42.0".to_string()],
            "rig-core must stay 0.42.0"
        );
        assert_eq!(
            package_versions(lock, "rig-agent"),
            vec!["0.42.0".to_string()],
            "rig-agent must stay 0.42.0"
        );
        assert_eq!(
            package_versions(lock, "rig-memory"),
            vec!["0.42.0".to_string()],
            "rig-memory must stay 0.42.0"
        );
        let rmcp = package_versions(lock, "rmcp");
        assert!(
            rmcp.iter().any(|v| v == "2.2.0"),
            "direct rmcp 2.2.0 missing from lock; found {rmcp:?}"
        );
        assert!(
            lock.contains("name = \"rustls\""),
            "rig rustls feature must pull rustls into the tree"
        );
    }

    fn package_versions(lock: &str, name: &str) -> Vec<String> {
        let mut versions = Vec::new();
        let mut current_name: Option<&str> = None;
        for line in lock.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("name = \"") {
                current_name = rest.strip_suffix('"');
            } else if let Some(rest) = line.strip_prefix("version = \"") {
                if current_name == Some(name) {
                    if let Some(version) = rest.strip_suffix('"') {
                        versions.push(version.to_string());
                    }
                }
            }
        }
        versions.sort();
        versions.dedup();
        versions
    }
}
