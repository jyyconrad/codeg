//! PR8 delivery record: compile matrix and live-gateway dogfood status.

/// Commands from `src-tauri/` that PR8 must pass. Run them in this order.
pub const COMPILE_MATRIX: &[&str] = &[
    "cargo check",
    "cargo test --features test-utils",
    "cargo clippy --all-targets --features test-utils -- -D warnings",
    "cargo check --no-default-features --bin codeg-server",
    "cargo test --no-default-features --bin codeg-server --lib",
    "cargo clippy --no-default-features --bin codeg-server --lib -- -D warnings",
    "cargo check --no-default-features --bin codeg-mcp",
    "cargo clippy --no-default-features --bin codeg-mcp -- -D warnings",
];

/// Live Chat Completions gateway dogfood. `pending` until desktop and server
/// each have a recorded run against a real streaming-tool gateway.
pub const DOGFOOD_STATUS: &str = "pending";

pub const DOGFOOD_REASON: &str = "No Chat Completions streaming-tool gateway was available; desktop and server live dogfood are not recorded.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_matrix_covers_desktop_server_and_mcp() {
        let joined = COMPILE_MATRIX.join("\n");
        assert!(joined.contains("cargo check"));
        assert!(joined.contains("cargo test --features test-utils"));
        assert!(joined.contains("clippy --all-targets --features test-utils"));
        assert!(joined.contains("--no-default-features --bin codeg-server"));
        assert!(joined.contains("--no-default-features --bin codeg-mcp"));
        assert_eq!(COMPILE_MATRIX.len(), 8);
    }

    #[test]
    fn live_gateway_dogfood_is_pending_without_a_gateway() {
        assert_eq!(DOGFOOD_STATUS, "pending");
        assert!(DOGFOOD_REASON.contains("gateway"));
    }
}
