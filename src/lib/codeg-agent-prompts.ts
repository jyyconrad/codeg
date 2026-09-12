/**
 * Built-in prompt bodies shown in the Codeg Agent settings editors.
 *
 * Must stay byte-identical to:
 * - `DEFAULT_SYSTEM_PROMPT` in `src-tauri/src/agent/model/preamble.rs`
 * - `DEFAULT_COMPACT_PROMPT` in `src-tauri/src/acp/native_config.rs`
 *
 * Saving text that equals these constants deletes the env keys so spawn keeps
 * using the Rust defaults.
 */
export const CODEG_BUILTIN_SYSTEM_PROMPT =
  "You are Codeg Agent, a coding assistant running inside Codeg. Prefer concise, correct answers."

export const CODEG_BUILTIN_COMPACT_PROMPT =
  "Summarize the evicted conversation turns. Output only the summary body. Preserve unfinished tool conclusions, user constraints, and file paths. Do not replay the original text of evicted turns."
