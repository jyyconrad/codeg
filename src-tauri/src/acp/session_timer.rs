//! Session timer domain — parse args, wake text, and ack rendering for
//! `set_session_timer`. Live scheduling lives on ConnectionManager (later).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_TIMER_SECONDS: u32 = 1800;
pub const WAKE_PREFIX: &str = "[session timer]";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTimerSpec {
    pub seconds: u32,
    pub reason: Option<String>,
    pub cancel_on_user_message: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTimerAck {
    pub ok: bool,
    pub timer_id: Option<String>,
    pub seconds: u32,
    pub cancel_on_user_message: bool,
    pub replaced: bool,
    pub error: Option<String>,
}

impl SessionTimerAck {
    pub fn connection_not_found() -> Self {
        Self {
            ok: false,
            timer_id: None,
            seconds: 0,
            cancel_on_user_message: true,
            replaced: false,
            error: Some("connection_not_found".into()),
        }
    }
}

pub fn parse_timer_args(args: &Value) -> Result<SessionTimerSpec, String> {
    let seconds = match args.get("seconds") {
        Some(Value::Number(n)) => {
            // Only whole JSON integers — reject floats via as_u64.
            n.as_u64()
                .ok_or_else(|| "seconds must be an integer".to_string())?
        }
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
                return Err("seconds must be a decimal integer string".into());
            }
            trimmed
                .parse::<u64>()
                .map_err(|_| "seconds must be a decimal integer string".to_string())?
        }
        Some(_) => return Err("seconds must be an integer or digit string".into()),
        None => return Err("seconds is required".into()),
    };

    if seconds < 1 || seconds > u64::from(MAX_TIMER_SECONDS) {
        return Err(format!(
            "seconds must be between 1 and {MAX_TIMER_SECONDS}"
        ));
    }

    let reason = match args.get("reason") {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    };

    let cancel_on_user_message = match args.get("cancel_on_user_message") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err("cancel_on_user_message must be a boolean".into());
        }
    };

    Ok(SessionTimerSpec {
        seconds: seconds as u32,
        reason,
        cancel_on_user_message,
    })
}

pub fn wake_prompt_text(spec: &SessionTimerSpec) -> String {
    match &spec.reason {
        Some(reason) => format!(
            "{WAKE_PREFIX} Scheduled wake.\n\nWaited: {} seconds.\nReason: {reason}\n\nThis is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on.",
            spec.seconds
        ),
        None => format!(
            "{WAKE_PREFIX} Scheduled wake.\n\nWaited: {} seconds.\n\nThis is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on.",
            spec.seconds
        ),
    }
}

pub fn timer_ack_text(ack: &SessionTimerAck) -> String {
    if ack.replaced {
        format!(
            "Timer set: wake this session in {}s. Previous timer replaced.",
            ack.seconds
        )
    } else {
        format!("Timer set: wake this session in {}s.", ack.seconds)
    }
}

pub fn render_timer_ack(ack: &SessionTimerAck) -> Value {
    json!({
        "content": [{ "type": "text", "text": timer_ack_text(ack) }],
        "isError": !ack.ok,
        "structuredContent": ack,
    })
}

#[async_trait]
pub trait SessionTimerAccess: Send + Sync {
    async fn set_timer(&self, parent_connection_id: &str, spec: SessionTimerSpec) -> SessionTimerAck;
    async fn cancel_by_parent(&self, parent_connection_id: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_integer_seconds_and_defaults_cancel_true() {
        let spec = parse_timer_args(&serde_json::json!({"seconds": 120})).unwrap();
        assert_eq!(spec.seconds, 120);
        assert_eq!(spec.reason, None);
        assert!(spec.cancel_on_user_message);
    }

    #[test]
    fn parse_accepts_numeric_string_seconds_and_false_flag() {
        let spec = parse_timer_args(&serde_json::json!({
            "seconds": "1",
            "reason": "  check CI  ",
            "cancel_on_user_message": false
        }))
        .unwrap();
        assert_eq!(spec.seconds, 1);
        assert_eq!(spec.reason.as_deref(), Some("check CI"));
        assert!(!spec.cancel_on_user_message);
    }

    #[test]
    fn parse_rejects_out_of_range_and_non_integer() {
        for args in [
            serde_json::json!({}),
            serde_json::json!({"seconds": 0}),
            serde_json::json!({"seconds": 1801}),
            serde_json::json!({"seconds": 1.5}),
            serde_json::json!({"seconds": true}),
            serde_json::json!({"seconds": 10, "cancel_on_user_message": "yes"}),
        ] {
            assert!(parse_timer_args(&args).is_err(), "{args}");
        }
    }

    #[test]
    fn wake_prompt_with_and_without_reason_is_stable() {
        let with = SessionTimerSpec {
            seconds: 90,
            reason: Some("poll deploy".into()),
            cancel_on_user_message: true,
        };
        assert_eq!(
            wake_prompt_text(&with),
            "[session timer] Scheduled wake.\n\nWaited: 90 seconds.\nReason: poll deploy\n\nThis is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on."
        );
        let without = SessionTimerSpec {
            seconds: 90,
            reason: None,
            cancel_on_user_message: true,
        };
        assert_eq!(
            wake_prompt_text(&without),
            "[session timer] Scheduled wake.\n\nWaited: 90 seconds.\n\nThis is a scheduled wake of this same session, not a new user request. Continue the work you were waiting on."
        );
        assert!(wake_prompt_text(&with).starts_with(WAKE_PREFIX));
    }

    #[test]
    fn ack_text_mentions_replace() {
        let mut ack = SessionTimerAck {
            ok: true,
            timer_id: Some("t1".into()),
            seconds: 12,
            cancel_on_user_message: true,
            replaced: false,
            error: None,
        };
        assert_eq!(timer_ack_text(&ack), "Timer set: wake this session in 12s.");
        ack.replaced = true;
        assert_eq!(
            timer_ack_text(&ack),
            "Timer set: wake this session in 12s. Previous timer replaced."
        );
    }
}
