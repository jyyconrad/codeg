//! Project standard Rig messages onto existing [`MessageTurn`] display types.
//!
//! Display is a projection, not an execution-history source. Tool-result-only
//! user messages are folded onto matching assistant tool cards.

use std::collections::HashMap;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use rig::completion::message::{
    AssistantContent, DocumentSourceKind, Image, MimeType, ToolResult, ToolResultContent,
    UserContent,
};
use rig::completion::Message;
use serde_json::Value;

use super::store::ExecutionFact;
use super::transcript::{acp_status_for, ToolOutcome, ToolPhase};
use crate::models::message::{ContentBlock, ImageData, MessageTurn, TurnRole};

/// Options for projecting messages into UI turns.
#[derive(Debug, Clone)]
pub struct DisplayOptions {
    pub session_id: String,
    /// Optional execution facts keyed by `tool_call_id` for progress/status on cards.
    pub facts: Vec<ExecutionFact>,
}

/// Convert stored Rig messages (plus optional runtime facts) into UI turns.
pub fn messages_to_turns(messages: &[Message], opts: &DisplayOptions) -> Vec<MessageTurn> {
    let mut turns: Vec<MessageTurn> = Vec::new();
    let mut seq = 0usize;

    for message in messages {
        match message {
            Message::User { content } => {
                handle_user_message(&mut turns, &mut seq, &opts.session_id, content);
            }
            Message::Assistant { id, content } => {
                let blocks = assistant_blocks(content);
                if blocks.is_empty() {
                    continue;
                }
                seq += 1;
                turns.push(new_turn(
                    turn_id(&opts.session_id, seq),
                    TurnRole::Assistant,
                    blocks,
                    id.clone(),
                ));
            }
            Message::System { content } => {
                if content.is_empty() {
                    continue;
                }
                seq += 1;
                turns.push(new_turn(
                    turn_id(&opts.session_id, seq),
                    TurnRole::System,
                    vec![ContentBlock::Text {
                        text: content.clone(),
                    }],
                    None,
                ));
            }
        }
    }

    overlay_facts(&mut turns, &opts.facts);
    turns
}

fn handle_user_message(
    turns: &mut Vec<MessageTurn>,
    seq: &mut usize,
    session_id: &str,
    content: &[UserContent],
) {
    let (blocks, results) = user_display_parts(content);
    let mut unmatched = Vec::new();
    for result in results {
        if !attach_result(turns, result.clone()) {
            unmatched.push(result);
        }
    }

    if is_tool_result_only(content) && unmatched.is_empty() {
        return;
    }

    if is_tool_result_only(content) {
        *seq += 1;
        let mut turn = new_turn(
            turn_id(session_id, *seq),
            TurnRole::Assistant,
            Vec::new(),
            None,
        );
        for result in unmatched {
            upsert_tool_result(&mut turn, result, false);
        }
        turns.push(turn);
        return;
    }

    if !blocks.is_empty() {
        *seq += 1;
        let role = if is_history_summary(&joined_user_text(content)) {
            TurnRole::System
        } else {
            TurnRole::User
        };
        turns.push(new_turn(turn_id(session_id, *seq), role, blocks, None));
    }

    if !unmatched.is_empty() {
        *seq += 1;
        let mut turn = new_turn(
            turn_id(session_id, *seq),
            TurnRole::Assistant,
            Vec::new(),
            None,
        );
        for result in unmatched {
            upsert_tool_result(&mut turn, result, false);
        }
        turns.push(turn);
    }
}

fn turn_id(session_id: &str, seq: usize) -> String {
    format!("{session_id}:m{seq}")
}

fn new_turn(
    id: String,
    role: TurnRole,
    blocks: Vec<ContentBlock>,
    agent_message_id: Option<String>,
) -> MessageTurn {
    MessageTurn {
        id,
        role,
        blocks,
        timestamp: DateTime::<Utc>::UNIX_EPOCH,
        usage: None,
        duration_ms: None,
        model: None,
        completed_at: None,
        agent_message_id,
    }
}

fn is_tool_result_only(content: &[UserContent]) -> bool {
    !content.is_empty()
        && content
            .iter()
            .all(|part| matches!(part, UserContent::ToolResult(_)))
}

fn joined_user_text(content: &[UserContent]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            UserContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_history_summary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    trimmed.starts_with("历史摘要：")
        || trimmed.starts_with("历史摘要:")
        || trimmed.starts_with("Conversation summary:")
        || trimmed.starts_with("Conversation summary")
        || lower.starts_with("history summary:")
        || lower.starts_with("history summary")
}

#[derive(Clone)]
struct FoldedResult {
    call_id: String,
    output: String,
    images: Vec<ImageData>,
}

fn user_display_parts(content: &[UserContent]) -> (Vec<ContentBlock>, Vec<FoldedResult>) {
    let mut blocks = Vec::new();
    let mut results = Vec::new();
    for part in content {
        match part {
            UserContent::Text(text) if !text.text.is_empty() => {
                blocks.push(ContentBlock::Text {
                    text: text.text.clone(),
                });
            }
            UserContent::Text(_) => {}
            UserContent::ToolResult(result) => results.push(folded_from_tool_result(result)),
            UserContent::Image(image) => {
                if let Some(block) = image_block(image) {
                    blocks.push(block);
                }
            }
            UserContent::Document(doc) => {
                blocks.push(source_label("document", &doc.data));
            }
            UserContent::Audio(audio) => {
                blocks.push(source_label("audio", &audio.data));
            }
            UserContent::Video(video) => {
                blocks.push(source_label("video", &video.data));
            }
        }
    }
    (blocks, results)
}

fn assistant_blocks(content: &[AssistantContent]) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    for part in content {
        match part {
            AssistantContent::Text(text) if !text.text.is_empty() => {
                blocks.push(ContentBlock::Text {
                    text: text.text.clone(),
                });
            }
            AssistantContent::Text(_) => {}
            AssistantContent::ToolCall(call) => {
                blocks.push(ContentBlock::ToolUse {
                    tool_use_id: Some(call.id.as_str().to_string()),
                    tool_name: call.function.name.clone(),
                    input_preview: input_preview(&call.function.arguments),
                    status: None,
                    meta: None,
                });
            }
            AssistantContent::Reasoning(reasoning) => {
                let text = reasoning.display_text();
                if !text.is_empty() {
                    blocks.push(ContentBlock::Thinking { text });
                }
            }
            AssistantContent::Image(image) => {
                if let Some(block) = image_block(image) {
                    blocks.push(block);
                }
            }
        }
    }
    blocks
}

fn input_preview(args: &Value) -> Option<String> {
    if args.is_null() {
        None
    } else {
        Some(args.to_string())
    }
}

fn folded_from_tool_result(result: &ToolResult) -> FoldedResult {
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for item in &result.content {
        match item {
            ToolResultContent::Text(text) => texts.push(text.text.clone()),
            ToolResultContent::Json { value } => texts.push(value.to_string()),
            ToolResultContent::Image(image) => {
                if let Some(data) = image_parts(image) {
                    images.push(data);
                }
            }
        }
    }
    FoldedResult {
        call_id: result.call.as_str().to_string(),
        output: texts.join("\n"),
        images,
    }
}

fn attach_result(turns: &mut [MessageTurn], result: FoldedResult) -> bool {
    let Some(idx) = find_result_turn(turns, &result.call_id) else {
        return false;
    };
    upsert_tool_result(&mut turns[idx], result, false);
    true
}

fn find_result_turn(turns: &[MessageTurn], call_id: &str) -> Option<usize> {
    turns
        .iter()
        .position(|turn| {
            turn.blocks.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::ToolUse {
                        tool_use_id: Some(id),
                        ..
                    } if id == call_id
                )
            })
        })
        .or_else(|| {
            turns
                .iter()
                .rposition(|turn| matches!(turn.role, TurnRole::Assistant))
        })
}

fn upsert_tool_result(turn: &mut MessageTurn, result: FoldedResult, is_error: bool) {
    let FoldedResult {
        call_id,
        output,
        images,
    } = result;
    for block in &mut turn.blocks {
        if let ContentBlock::ToolResult {
            tool_use_id: Some(id),
            output_preview,
            is_error: err,
            images: existing_images,
            ..
        } = block
        {
            if id == &call_id {
                if !output.is_empty() {
                    *output_preview = Some(output);
                }
                if !images.is_empty() {
                    *existing_images = images;
                }
                *err = is_error;
                return;
            }
        }
    }
    let output_preview = if output.is_empty() {
        None
    } else {
        Some(output)
    };
    turn.blocks.push(ContentBlock::ToolResult {
        tool_use_id: Some(call_id),
        output_preview,
        is_error,
        agent_stats: None,
        images,
    });
}

fn overlay_facts(turns: &mut [MessageTurn], facts: &[ExecutionFact]) {
    if facts.is_empty() {
        return;
    }
    let mut by_id: HashMap<&str, &ExecutionFact> = HashMap::new();
    for fact in facts {
        by_id.insert(fact.tool_call_id.as_str(), fact);
    }
    for turn in turns {
        overlay_turn(turn, &by_id);
    }
}

fn overlay_turn(turn: &mut MessageTurn, facts: &HashMap<&str, &ExecutionFact>) {
    let ids: Vec<String> = turn
        .blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolUse {
                tool_use_id: Some(id),
                ..
            } => Some(id.clone()),
            _ => None,
        })
        .collect();

    for id in ids {
        let Some(fact) = facts.get(id.as_str()).copied() else {
            continue;
        };
        let status = acp_status_for(fact.outcome, fact.phase).to_string();
        for block in &mut turn.blocks {
            if let ContentBlock::ToolUse {
                tool_use_id: Some(uid),
                status: st,
                ..
            } = block
            {
                if uid == &id {
                    *st = Some(status.clone());
                }
            }
        }

        let is_error = !matches!(fact.outcome, Some(ToolOutcome::Success));
        let output = fact_outcome_text(fact);
        let mut found = false;
        for block in &mut turn.blocks {
            if let ContentBlock::ToolResult {
                tool_use_id: Some(uid),
                output_preview,
                is_error: err,
                ..
            } = block
            {
                if uid == &id {
                    found = true;
                    if let Some(text) = output.clone() {
                        *output_preview = Some(text);
                    }
                    *err = is_error;
                }
            }
        }
        if !found && should_materialize_result(fact) {
            turn.blocks.push(ContentBlock::ToolResult {
                tool_use_id: Some(id),
                output_preview: output,
                is_error,
                agent_stats: None,
                images: Vec::new(),
            });
        }
    }
}

fn fact_outcome_text(fact: &ExecutionFact) -> Option<String> {
    match fact.outcome {
        Some(ToolOutcome::Success) => Some(
            fact.model_presentation
                .clone()
                .unwrap_or_else(|| fact.outcome_feedback()),
        ),
        Some(_) => Some(fact.outcome_feedback()),
        None if fact.phase == ToolPhase::Terminal => Some(fact.outcome_feedback()),
        None => fact.model_presentation.clone(),
    }
}

fn should_materialize_result(fact: &ExecutionFact) -> bool {
    matches!(fact.phase, ToolPhase::Terminal) || fact.outcome.is_some()
}

fn image_block(image: &Image) -> Option<ContentBlock> {
    image_parts(image).map(|img| ContentBlock::Image {
        data: img.data,
        mime_type: img.mime_type,
        uri: img.uri,
    })
}

fn image_parts(image: &Image) -> Option<ImageData> {
    let mime_type = image
        .media_type
        .as_ref()
        .map(MimeType::to_mime_type)
        .unwrap_or("image/png")
        .to_string();
    match &image.data {
        DocumentSourceKind::Base64(data) | DocumentSourceKind::String(data) => Some(ImageData {
            data: data.clone(),
            mime_type,
            uri: None,
        }),
        DocumentSourceKind::Url(url) => Some(ImageData {
            data: String::new(),
            mime_type,
            uri: Some(url.clone()),
        }),
        DocumentSourceKind::FileId(id) => Some(ImageData {
            data: String::new(),
            mime_type,
            uri: Some(id.clone()),
        }),
        DocumentSourceKind::Raw(bytes) => Some(ImageData {
            data: BASE64.encode(bytes),
            mime_type,
            uri: None,
        }),
        DocumentSourceKind::Unknown => None,
    }
}

fn source_label(kind: &str, data: &DocumentSourceKind) -> ContentBlock {
    let text = match data {
        DocumentSourceKind::Url(url) => format!("[{kind}]({url})"),
        DocumentSourceKind::FileId(id) => format!("[{kind}:{id}]"),
        _ => format!("[{kind}]"),
    };
    ContentBlock::Text { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::completion::message::AssistantContent;
    use serde_json::json;

    fn opts(session_id: &str) -> DisplayOptions {
        DisplayOptions {
            session_id: session_id.into(),
            facts: Vec::new(),
        }
    }

    fn tool_calls(ids: &[&str], name: &str) -> Message {
        Message::Assistant {
            id: None,
            content: ids
                .iter()
                .map(|id| AssistantContent::tool_call(*id, name, json!({"id": id})))
                .collect(),
        }
    }

    fn tool_results_msg(ids: &[&str], name: &str) -> Message {
        Message::User {
            content: ids
                .iter()
                .map(|id| {
                    UserContent::tool_result(
                        *id,
                        name,
                        vec![ToolResultContent::text(format!("out-{id}"))],
                    )
                })
                .collect(),
        }
    }

    fn texts(turn: &MessageTurn) -> Vec<&str> {
        turn.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_use_ids(turn: &MessageTurn) -> Vec<&str> {
        turn.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse {
                    tool_use_id: Some(id),
                    ..
                } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_result_ids(turn: &MessageTurn) -> Vec<&str> {
        turn.blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id: Some(id),
                    ..
                } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_use_status<'a>(turn: &'a MessageTurn, call_id: &str) -> Option<&'a str> {
        turn.blocks.iter().find_map(|block| match block {
            ContentBlock::ToolUse {
                tool_use_id: Some(id),
                status,
                ..
            } if id == call_id => status.as_deref(),
            _ => None,
        })
    }

    fn tool_result_block<'a>(
        turn: &'a MessageTurn,
        call_id: &str,
    ) -> Option<(&'a Option<String>, bool)> {
        turn.blocks.iter().find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id: Some(id),
                output_preview,
                is_error,
                ..
            } if id == call_id => Some((output_preview, *is_error)),
            _ => None,
        })
    }

    #[test]
    fn user_and_assistant_text_become_two_turns() {
        let messages = vec![Message::user("hello"), Message::assistant("world")];
        let turns = messages_to_turns(&messages, &opts("sess"));
        assert_eq!(turns.len(), 2);
        assert!(matches!(turns[0].role, TurnRole::User));
        assert!(matches!(turns[1].role, TurnRole::Assistant));
        assert_eq!(texts(&turns[0]), vec!["hello"]);
        assert_eq!(texts(&turns[1]), vec!["world"]);
        assert_eq!(turns[0].id, "sess:m1");
        assert_eq!(turns[1].id, "sess:m2");
    }

    #[test]
    fn tool_result_user_message_folds_into_assistant_turn() {
        let messages = vec![
            tool_calls(&["call_a"], "bash"),
            Message::tool_result("call_a", "bash", "exit 0"),
        ];
        let turns = messages_to_turns(&messages, &opts("sess"));
        assert_eq!(turns.len(), 1);
        assert!(matches!(turns[0].role, TurnRole::Assistant));
        assert!(!turns.iter().any(|t| matches!(t.role, TurnRole::User)));
        assert_eq!(tool_use_ids(&turns[0]), vec!["call_a"]);
        assert_eq!(tool_result_ids(&turns[0]), vec!["call_a"]);
        let (preview, is_error) = tool_result_block(&turns[0], "call_a").expect("result");
        assert_eq!(preview.as_deref(), Some("exit 0"));
        assert!(!is_error);
    }

    #[test]
    fn three_tool_rounds_2_4_3_preserve_all_call_ids() {
        let round1 = ["c1", "c2"];
        let round2 = ["c3", "c4", "c5", "c6"];
        let round3 = ["c7", "c8", "c9"];
        let messages = vec![
            tool_calls(&round1, "write_mem"),
            tool_results_msg(&round1, "write_mem"),
            tool_calls(&round2, "write_mem"),
            tool_results_msg(&round2, "write_mem"),
            tool_calls(&round3, "write_mem"),
            tool_results_msg(&round3, "write_mem"),
        ];
        let turns = messages_to_turns(&messages, &opts("s1"));
        assert_eq!(turns.len(), 3);
        assert!(turns
            .iter()
            .all(|turn| matches!(turn.role, TurnRole::Assistant)));
        assert!(!turns.iter().any(|t| matches!(t.role, TurnRole::User)));

        let expected = [round1.as_slice(), round2.as_slice(), round3.as_slice()];
        for (i, ids) in expected.into_iter().enumerate() {
            assert_eq!(tool_use_ids(&turns[i]), ids, "round {i} tool uses");
            assert_eq!(tool_result_ids(&turns[i]), ids, "round {i} tool results");
        }

        let all_ids: Vec<&str> = turns.iter().flat_map(tool_use_ids).collect();
        assert_eq!(
            all_ids,
            vec!["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8", "c9"]
        );
        assert_eq!(turns[0].id, "s1:m1");
        assert_eq!(turns[1].id, "s1:m2");
        assert_eq!(turns[2].id, "s1:m3");
    }

    #[test]
    fn history_summary_is_not_a_user_prompt() {
        let messages = vec![
            Message::user("历史摘要：已检查改动并通过相关验证。"),
            Message::user("History summary: prior work is done."),
            Message::user("Conversation summary: remaining docs."),
            Message::user("please continue"),
        ];
        let turns = messages_to_turns(&messages, &opts("sess"));
        assert_eq!(turns.len(), 4);
        assert!(matches!(turns[0].role, TurnRole::System));
        assert!(matches!(turns[1].role, TurnRole::System));
        assert!(matches!(turns[2].role, TurnRole::System));
        assert!(matches!(turns[3].role, TurnRole::User));
        assert_eq!(texts(&turns[3]), vec!["please continue"]);
        assert_eq!(
            turns
                .iter()
                .filter(|t| matches!(t.role, TurnRole::User))
                .count(),
            1
        );
    }

    #[test]
    fn unknown_fact_overlay_does_not_become_success() {
        let messages = vec![
            tool_calls(&["call_u"], "bash"),
            Message::tool_result("call_u", "bash", "ok"),
        ];
        let mut fact = ExecutionFact::pending("sess:1", "call_u", "bash", json!({}));
        fact.phase = ToolPhase::Terminal;
        fact.outcome = Some(ToolOutcome::Unknown);
        fact.executed = None;
        fact.model_presentation = Some("ok".into());
        let opts = DisplayOptions {
            session_id: "sess".into(),
            facts: vec![fact],
        };
        let turns = messages_to_turns(&messages, &opts);
        assert_eq!(turns.len(), 1);
        let status = tool_use_status(&turns[0], "call_u");
        assert_ne!(status, Some("completed"));
        assert_eq!(status, Some("failed"));
        let (preview, is_error) = tool_result_block(&turns[0], "call_u").expect("result");
        assert!(is_error);
        let body = preview.as_deref().unwrap_or("");
        assert_ne!(body, "ok");
        assert!(
            body.contains("could not be confirmed") || body.contains("not replayed"),
            "unknown overlay body: {body}"
        );
    }
}
