//! Rig Agent / Runner assembly for one Prompt. Session shell must not own this.

use futures::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::client::AgentClientExt;
use rig::completion::Message;
use rig::providers::openai::CompletionsClient;
use rig::tool::DynamicTool;
use tokio_util::sync::CancellationToken;

use crate::agent::hook::CodegHook;
use crate::agent::tools::{
    BashTool, EchoTool, EditFileTool, GlobTool, GrepTool, ReadFileTool, SkillTool, SubagentTool,
    UpdatePlanTool, WriteFileTool,
};

use super::{DEFAULT_INVALID_TOOL_CALL_RETRIES, DEFAULT_MAX_TURNS, DEFAULT_TOOL_CONCURRENCY};

pub enum NativeTurnOutcome {
    Complete,
    Cancelled,
    Failed(String),
}

pub struct NativeTurnTools {
    pub read: ReadFileTool,
    pub write: WriteFileTool,
    pub edit: EditFileTool,
    pub glob: GlobTool,
    pub grep: GrepTool,
    pub bash: BashTool,
    pub skill: SkillTool,
    pub plan: UpdatePlanTool,
    pub subagent: SubagentTool,
    pub echo: Option<EchoTool>,
    pub dynamic: Vec<DynamicTool>,
}

pub struct NativeTurnRequest {
    pub client: CompletionsClient,
    pub model_id: String,
    pub preamble: String,
    pub prompt: Message,
    pub tools: NativeTurnTools,
    pub hook: CodegHook,
    pub cancel: CancellationToken,
}

/// Build the Agent, start a Runner, and drain the stream.
///
/// Cancel during `stream()` drops the provider wait (no tools have started).
/// Once the stream exists, in-flight tools are polled to completion so MCP
/// cancel/retire and started file writes are not dropped.
pub async fn run_native_turn(request: NativeTurnRequest) -> NativeTurnOutcome {
    let NativeTurnRequest {
        client,
        model_id,
        preamble,
        prompt,
        tools,
        hook,
        cancel,
    } = request;
    let stream_fut = assemble_and_stream(client, model_id, preamble, prompt, tools, hook);
    let stream = tokio::select! {
        _ = cancel.cancelled() => return NativeTurnOutcome::Cancelled,
        stream = stream_fut => stream,
    };
    drain_native_stream(stream, cancel).await
}

async fn assemble_and_stream(
    client: CompletionsClient,
    model_id: String,
    preamble: String,
    prompt: Message,
    tools: NativeTurnTools,
    hook: CodegHook,
) -> rig::agent::StreamingResult {
    let NativeTurnTools {
        read,
        write,
        edit,
        glob,
        grep,
        bash,
        skill,
        plan,
        subagent,
        echo,
        dynamic,
    } = tools;
    let builder = client
        .agent(&model_id)
        .preamble(&preamble)
        .default_max_turns(DEFAULT_MAX_TURNS)
        .tool(read)
        .tool(write)
        .tool(edit)
        .tool(glob)
        .tool(grep)
        .tool(bash)
        .tool(skill)
        .tool(plan)
        .tool(subagent);
    let builder = if let Some(echo) = echo {
        builder.tool(echo)
    } else {
        builder
    };
    builder
        .dynamic_tools(dynamic)
        .build()
        .runner(prompt)
        .history(Vec::<Message>::new())
        .max_turns(DEFAULT_MAX_TURNS)
        .tool_concurrency(DEFAULT_TOOL_CONCURRENCY)
        .max_invalid_tool_call_retries(DEFAULT_INVALID_TOOL_CALL_RETRIES)
        .add_hook(hook)
        .stream()
        .await
}

async fn drain_native_stream(
    mut stream: rig::agent::StreamingResult,
    cancel: CancellationToken,
) -> NativeTurnOutcome {
    loop {
        match stream.next().await {
            None => {
                return if cancel.is_cancelled() {
                    NativeTurnOutcome::Cancelled
                } else {
                    NativeTurnOutcome::Complete
                };
            }
            Some(Ok(MultiTurnStreamItem::FinalResponse(_))) => {
                return if cancel.is_cancelled() {
                    NativeTurnOutcome::Cancelled
                } else {
                    NativeTurnOutcome::Complete
                };
            }
            Some(Err(err)) => {
                let message = err.to_string();
                if cancel.is_cancelled() || message.to_ascii_lowercase().contains("cancel") {
                    return NativeTurnOutcome::Cancelled;
                }
                return NativeTurnOutcome::Failed(message);
            }
            Some(Ok(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use rig::completion::message::{ImageMediaType, UserContent};
    use rig::completion::Message;
    use rig::tool::Tool;

    use crate::agent::tools::{SubagentTool, UpdatePlanTool};

    #[test]
    fn native_turn_tools_include_update_plan_and_subagent() {
        assert_eq!(UpdatePlanTool::NAME, "update_plan");
        assert_eq!(SubagentTool::NAME, "subagent");
    }

    #[test]
    fn native_turn_prompt_is_a_user_message_with_image() {
        let prompt = Message::from(vec![
            UserContent::text("describe this"),
            UserContent::image_base64("iVBORw0KGgo=", Some(ImageMediaType::PNG), None),
        ]);
        let Message::User { content } = prompt else {
            panic!("expected user message");
        };
        assert_eq!(content.len(), 2);
        assert!(matches!(content[1], UserContent::Image(_)));
    }
}
