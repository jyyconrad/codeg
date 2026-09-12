pub mod builtin_skills;
pub mod code_intel;
pub mod context;
pub mod delivery;
pub mod hook;
pub mod mode;
pub mod model;
pub mod session;
pub mod tools;

pub use hook::CodegHook;
pub use model::{
    completions_client, CompletionsClientError, DEFAULT_INVALID_TOOL_CALL_RETRIES,
    DEFAULT_MAX_TURNS, DEFAULT_TOOL_CONCURRENCY,
};
pub use session::{
    inspect_native_prompt, run_native_session, spawn_native_session, NativePromptView,
    NativeSessionArgs,
};

#[cfg(test)]
mod contracts;
