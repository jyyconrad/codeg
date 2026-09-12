//! Rig Agent / Runner assembly for one Prompt. Session shell must not own this.

use futures::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::client::AgentClientExt;
use rig::completion::Message;
use rig::tool::DynamicTool;
use tokio_util::sync::CancellationToken;

use crate::agent::hook::CodegHook;
use crate::agent::tools::{
    BashTool, CodegraphTool, EchoTool, EditFileTool, EnterPlanModeTool, ExitPlanModeTool, GlobTool,
    GrepTool, LspTool, ReadFileTool, RecallTool, SkillTool, SubagentTool, UpdatePlanTool,
    WriteExploreReportTool, WriteFileTool, WritePlanTool,
};

use super::{CodegLlmClient, DEFAULT_INVALID_TOOL_CALL_RETRIES, DEFAULT_TOOL_CONCURRENCY};

pub enum NativeTurnOutcome {
    Complete,
    Cancelled,
    Failed(String),
}

pub struct NativeTurnTools {
    pub read: ReadFileTool,
    pub recall: RecallTool,
    pub write: Option<WriteFileTool>,
    pub edit: Option<EditFileTool>,
    pub glob: GlobTool,
    pub grep: GrepTool,
    pub codegraph: Option<CodegraphTool>,
    pub lsp: Option<LspTool>,
    pub bash: Option<BashTool>,
    pub skill: SkillTool,
    pub plan: Option<UpdatePlanTool>,
    pub write_plan: Option<WritePlanTool>,
    pub enter_plan: Option<EnterPlanModeTool>,
    pub exit_plan: Option<ExitPlanModeTool>,
    pub write_explore: Option<WriteExploreReportTool>,
    pub subagent: Option<SubagentTool>,
    pub echo: Option<EchoTool>,
    pub dynamic: Vec<DynamicTool>,
}

pub struct NativeTurnRequest {
    pub client: CodegLlmClient,
    pub model_id: String,
    pub preamble: String,
    pub prompt: Message,
    pub tools: NativeTurnTools,
    pub hook: CodegHook,
    pub cancel: CancellationToken,
    pub max_turns: usize,
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
        max_turns,
    } = request;
    let stream = tokio::select! {
        _ = cancel.cancelled() => return NativeTurnOutcome::Cancelled,
        stream = async {
            match client {
                CodegLlmClient::Completions(client) => {
                    assemble_and_stream(client, model_id, preamble, prompt, tools, hook, max_turns)
                        .await
                }
                CodegLlmClient::Responses(client) => {
                    assemble_and_stream(client, model_id, preamble, prompt, tools, hook, max_turns)
                        .await
                }
            }
        } => stream,
    };
    drain_native_stream(stream, cancel).await
}

async fn assemble_and_stream<C>(
    client: C,
    model_id: String,
    preamble: String,
    prompt: Message,
    tools: NativeTurnTools,
    hook: CodegHook,
    max_turns: usize,
) -> rig::agent::StreamingResult
where
    C: AgentClientExt + Send,
    C::CompletionModel: 'static,
{
    let NativeTurnTools {
        read,
        recall,
        write,
        edit,
        glob,
        grep,
        codegraph,
        lsp,
        bash,
        skill,
        plan,
        write_plan,
        enter_plan,
        exit_plan,
        write_explore,
        subagent,
        echo,
        dynamic,
    } = tools;
    let mut builder = client
        .agent(&model_id)
        .preamble(&preamble)
        .default_max_turns(max_turns.max(1))
        .tool(read)
        .tool(recall)
        .tool(glob)
        .tool(grep)
        .tool(skill);
    if let Some(codegraph) = codegraph {
        builder = builder.tool(codegraph);
    }
    if let Some(lsp) = lsp {
        builder = builder.tool(lsp);
    }
    if let Some(write) = write {
        builder = builder.tool(write);
    }
    if let Some(edit) = edit {
        builder = builder.tool(edit);
    }
    if let Some(bash) = bash {
        builder = builder.tool(bash);
    }
    if let Some(plan) = plan {
        builder = builder.tool(plan);
    }
    if let Some(write_plan) = write_plan {
        builder = builder.tool(write_plan);
    }
    if let Some(enter_plan) = enter_plan {
        builder = builder.tool(enter_plan);
    }
    if let Some(exit_plan) = exit_plan {
        builder = builder.tool(exit_plan);
    }
    if let Some(write_explore) = write_explore {
        builder = builder.tool(write_explore);
    }
    if let Some(subagent) = subagent {
        builder = builder.tool(subagent);
    }
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
        .max_turns(max_turns.max(1))
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

    use crate::agent::tools::{CodegraphTool, LspTool, SubagentTool, UpdatePlanTool};

    #[test]
    fn native_turn_tools_include_update_plan_and_subagent() {
        assert_eq!(UpdatePlanTool::NAME, "update_plan");
        assert_eq!(SubagentTool::NAME, "subagent");
        assert_eq!(CodegraphTool::NAME, "codegraph");
        assert_eq!(LspTool::NAME, "lsp");
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
