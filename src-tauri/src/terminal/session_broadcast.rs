//! Turn a terminal-mode hook into the ACP events codeg already broadcasts.
//!
//! There is no ACP connection: a synthetic `acp://event` would only fill the
//! frontend's unmapped buffer, and registering a connection would replace the
//! live PTY. Status still reaches the sidebar through `conversation://changed`.
//! Typed envelopes go to [`crate::acp::internal_bus::InternalEventBus`] under
//! `terminal-hook:{terminal_id}`. Lifecycle ignores that id because it is not
//! a live connection. Pet still sees prompting, turn-complete, and disconnect.
//! Chat-channel skips the connection-less `TurnComplete` (it has no title or
//! body) and formats `TerminalSessionCard`, which names the conversation row.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::acp::types::{AcpEvent, ConnectionStatus, EventEnvelope};
use crate::db::entities::conversation::ConversationStatus;
use crate::db::service::conversation_service;
use crate::models::AgentType;
use crate::web::event_bridge::{
    emit_event, ConversationChange, EventEmitter, CONVERSATION_CHANGED_EVENT,
};

static TERMINAL_HOOK_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalObservation {
    SessionStarted,
    TurnStarted,
    TurnStopped,
    SessionEnded,
}

impl TerminalObservation {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "session_started" => Some(Self::SessionStarted),
            "turn_started" => Some(Self::TurnStarted),
            "turn_stopped" => Some(Self::TurnStopped),
            "session_ended" | "process_exit" => Some(Self::SessionEnded),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatusEffect {
    Set(ConversationStatus),
    CancelIfRunning,
}

#[derive(Debug, Clone)]
pub(crate) struct ObservationPlan {
    pub status: StatusEffect,
    pub events: Vec<AcpEvent>,
}

pub struct TerminalObservationPublish<'a> {
    pub conversation_id: i32,
    pub agent_type: AgentType,
    pub external_id: &'a str,
    pub title: Option<&'a str>,
    pub terminal_id: Option<&'a str>,
    pub observation: Option<&'a str>,
    pub notice_body: Option<&'a str>,
}

pub async fn publish_terminal_observation(
    emitter: &EventEmitter,
    conn: &sea_orm::DatabaseConnection,
    publish: TerminalObservationPublish<'_>,
) {
    let Some(raw) = publish
        .observation
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(kind) = TerminalObservation::parse(raw) else {
        tracing::warn!("[terminal-session] ignoring unknown observation {raw}");
        return;
    };
    let plan = plan_terminal_observation(
        kind,
        publish.conversation_id,
        publish.agent_type,
        publish.external_id,
        publish.title,
        publish.notice_body,
    );
    let wrote = apply_status(conn, publish.conversation_id, plan.status.clone()).await;
    if let Some(ref status) = wrote {
        if let Ok(serde_json::Value::String(status_str)) = serde_json::to_value(status) {
            emit_event(
                emitter,
                CONVERSATION_CHANGED_EVENT,
                ConversationChange::Status {
                    id: publish.conversation_id,
                    status: status_str,
                },
            );
        }
    }
    let Some(terminal_id) = publish
        .terminal_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let Some(bus) = emitter.acp_event_bus() else {
        return;
    };
    let connection_id = format!("terminal-hook:{terminal_id}");
    for payload in bus_events(&plan, wrote) {
        bus.send(Arc::new(EventEnvelope {
            seq: TERMINAL_HOOK_SEQ.fetch_add(1, Ordering::Relaxed),
            connection_id: connection_id.clone(),
            payload,
        }));
    }
}

pub(crate) fn plan_terminal_observation(
    kind: TerminalObservation,
    conversation_id: i32,
    agent_type: AgentType,
    external_id: &str,
    title: Option<&str>,
    notice_body: Option<&str>,
) -> ObservationPlan {
    let session_id = nonempty(Some(external_id)).map(str::to_string);
    let title = nonempty(title).map(str::to_string);
    let agent = agent_type.as_wire().into_owned();
    let mut events = Vec::new();
    let status = match kind {
        // The CLI is at its prompt. `create` inserts the row as InProgress,
        // which is the sidebar's running spinner; leaving that in place made
        // every newly opened terminal look busy, and a later process exit then
        // cancelled it. A turn is the only thing that is actually running.
        TerminalObservation::SessionStarted => {
            if let Some(session_id) = session_id.clone() {
                events.push(AcpEvent::SessionStarted { session_id });
            }
            if let Some(title) = title.clone() {
                events.push(AcpEvent::NativeSessionTitle { title });
            }
            events.push(AcpEvent::ConversationStatusChanged {
                conversation_id,
                status: ConversationStatus::PendingReview,
            });
            StatusEffect::Set(ConversationStatus::PendingReview)
        }
        TerminalObservation::TurnStarted => {
            if let Some(title) = title.clone() {
                events.push(AcpEvent::NativeSessionTitle { title });
            }
            events.push(AcpEvent::StatusChanged {
                status: ConnectionStatus::Prompting,
            });
            events.push(AcpEvent::ConversationStatusChanged {
                conversation_id,
                status: ConversationStatus::InProgress,
            });
            // A start card without text matches ACP: nothing is pushed.
            if let Some(body) = nonempty(notice_body).map(str::to_string) {
                events.push(AcpEvent::TerminalSessionCard {
                    conversation_id,
                    agent_type: agent.clone(),
                    kind: "turn_started".to_string(),
                    body: Some(body),
                });
            }
            StatusEffect::Set(ConversationStatus::InProgress)
        }
        TerminalObservation::TurnStopped => {
            if let Some(title) = title.clone() {
                events.push(AcpEvent::NativeSessionTitle { title });
            }
            if let Some(session_id) = session_id.clone() {
                events.push(AcpEvent::TurnComplete {
                    session_id,
                    stop_reason: "end_turn".to_string(),
                    agent_type: agent.clone(),
                    run_id: None,
                });
            }
            // The stop card still goes out when the hook had no assistant text.
            // Pet keeps the TurnComplete above; chat-channel ignores that one.
            events.push(AcpEvent::TerminalSessionCard {
                conversation_id,
                agent_type: agent,
                kind: "turn_stopped".to_string(),
                body: nonempty(notice_body).map(str::to_string),
            });
            events.push(AcpEvent::ConversationStatusChanged {
                conversation_id,
                status: ConversationStatus::PendingReview,
            });
            StatusEffect::Set(ConversationStatus::PendingReview)
        }
        TerminalObservation::SessionEnded => {
            events.push(AcpEvent::StatusChanged {
                status: ConnectionStatus::Disconnected,
            });
            events.push(AcpEvent::ConversationStatusChanged {
                conversation_id,
                status: ConversationStatus::Cancelled,
            });
            StatusEffect::CancelIfRunning
        }
    };
    ObservationPlan { status, events }
}

/// Drop `ConversationStatusChanged` unless the database write actually landed.
/// Disconnect still goes out so pet clears a prompting entry that will never
/// receive a later turn.
pub(crate) fn bus_events(
    plan: &ObservationPlan,
    wrote: Option<ConversationStatus>,
) -> Vec<AcpEvent> {
    plan.events
        .iter()
        .filter(|event| match event {
            AcpEvent::ConversationStatusChanged { status, .. } => wrote.as_ref() == Some(status),
            _ => true,
        })
        .cloned()
        .collect()
}

async fn apply_status(
    conn: &sea_orm::DatabaseConnection,
    conversation_id: i32,
    effect: StatusEffect,
) -> Option<ConversationStatus> {
    match effect {
        StatusEffect::Set(status) => {
            match conversation_service::update_status(conn, conversation_id, status.clone()).await {
                Ok(()) => Some(status),
                Err(error) => {
                    tracing::warn!(
                    "[terminal-session] status update for conversation {conversation_id} failed: {error}"
                );
                    None
                }
            }
        }
        StatusEffect::CancelIfRunning => {
            match conversation_service::update_status_if(
                conn,
                conversation_id,
                ConversationStatus::InProgress,
                ConversationStatus::Cancelled,
            )
            .await
            {
                Ok(true) => Some(ConversationStatus::Cancelled),
                Ok(false) => None,
                Err(error) => {
                    tracing::warn!(
                        "[terminal-session] session-end status for conversation {conversation_id} failed: {error}"
                    );
                    None
                }
            }
        }
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types_of(events: &[AcpEvent]) -> Vec<String> {
        events
            .iter()
            .map(|event| {
                serde_json::to_value(event).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn session_start_is_idle_and_does_not_look_like_a_running_turn() {
        let plan = plan_terminal_observation(
            TerminalObservation::SessionStarted,
            7,
            AgentType::ClaudeCode,
            "sid-1",
            Some("Plan the release"),
            None,
        );
        assert_eq!(
            plan.status,
            StatusEffect::Set(ConversationStatus::PendingReview)
        );
        assert_eq!(
            types_of(&plan.events),
            [
                "session_started",
                "native_session_title",
                "conversation_status_changed",
            ]
        );
        assert!(plan.events.iter().all(|event| !matches!(
            event,
            AcpEvent::StatusChanged {
                status: ConnectionStatus::Prompting
            }
        )));
        assert!(matches!(
            &plan.events[0],
            AcpEvent::SessionStarted { session_id } if session_id == "sid-1"
        ));
        assert!(matches!(
            &plan.events[2],
            AcpEvent::ConversationStatusChanged {
                conversation_id: 7,
                status: ConversationStatus::PendingReview
            }
        ));
    }

    #[test]
    fn turn_start_prompts_without_pretending_a_new_session() {
        let plan = plan_terminal_observation(
            TerminalObservation::TurnStarted,
            7,
            AgentType::Codex,
            "sid-1",
            None,
            Some("fix the pane"),
        );
        assert_eq!(
            types_of(&plan.events),
            [
                "status_changed",
                "conversation_status_changed",
                "terminal_session_card",
            ]
        );
        assert!(matches!(
            plan.events.last(),
            Some(AcpEvent::TerminalSessionCard {
                kind,
                body,
                conversation_id: 7,
                ..
            }) if kind == "turn_started" && body.as_deref() == Some("fix the pane")
        ));
        let quiet = plan_terminal_observation(
            TerminalObservation::TurnStarted,
            7,
            AgentType::Codex,
            "sid-1",
            None,
            None,
        );
        assert_eq!(
            types_of(&quiet.events),
            ["status_changed", "conversation_status_changed"]
        );
        assert!(matches!(
            &plan.events[0],
            AcpEvent::StatusChanged {
                status: ConnectionStatus::Prompting
            }
        ));
    }

    #[test]
    fn turn_stop_is_pending_review_not_completed() {
        let plan = plan_terminal_observation(
            TerminalObservation::TurnStopped,
            7,
            AgentType::Codex,
            "sid-1",
            Some("Keep the title"),
            Some("all done"),
        );
        assert_eq!(
            plan.status,
            StatusEffect::Set(ConversationStatus::PendingReview)
        );
        assert!(matches!(
            &plan.events[1],
            AcpEvent::TurnComplete { stop_reason, run_id, session_id, .. }
                if stop_reason == "end_turn" && run_id.is_none() && session_id == "sid-1"
        ));
        let wire = serde_json::to_value(&plan.events[1]).unwrap();
        assert!(wire.get("run_id").is_none());
        assert_eq!(wire["agent_type"], AgentType::Codex.as_wire().as_ref());
        assert!(matches!(
            plan.events.last(),
            Some(AcpEvent::ConversationStatusChanged {
                status: ConversationStatus::PendingReview,
                ..
            })
        ));
        assert!(plan.events.iter().any(|event| matches!(
            event,
            AcpEvent::TerminalSessionCard {
                kind,
                body,
                conversation_id: 7,
                ..
            } if kind == "turn_stopped" && body.as_deref() == Some("all done")
        )));
    }

    #[test]
    fn session_end_cancels_only_a_still_running_row_and_always_disconnects() {
        let plan = plan_terminal_observation(
            TerminalObservation::SessionEnded,
            7,
            AgentType::ClaudeCode,
            "sid-1",
            Some("ignored"),
            Some("not a card"),
        );
        assert_eq!(plan.status, StatusEffect::CancelIfRunning);
        assert_eq!(
            types_of(&plan.events),
            ["status_changed", "conversation_status_changed"]
        );
        assert!(matches!(
            &plan.events[0],
            AcpEvent::StatusChanged {
                status: ConnectionStatus::Disconnected
            }
        ));
        assert!(plan.events.iter().all(|event| !matches!(
            event,
            AcpEvent::NativeSessionTitle { .. } | AcpEvent::TurnComplete { .. }
        )));

        let suppressed = bus_events(&plan, None);
        assert_eq!(types_of(&suppressed), ["status_changed"]);
        let published = bus_events(&plan, Some(ConversationStatus::Cancelled));
        assert_eq!(
            types_of(&published),
            ["status_changed", "conversation_status_changed"]
        );
    }

    #[test]
    fn unknown_observations_are_ignored() {
        assert_eq!(
            TerminalObservation::parse("session_ended"),
            Some(TerminalObservation::SessionEnded)
        );
        assert_eq!(
            TerminalObservation::parse("process_exit"),
            Some(TerminalObservation::SessionEnded)
        );
        assert!(TerminalObservation::parse("completed").is_none());
    }
}
