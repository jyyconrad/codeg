import type {
  AgentType,
  ContentBlock,
  MessageTurn,
  PromptDraft,
  PromptInputBlock,
} from "@/lib/types"

/**
 * Agents whose `session/fork` honours `_meta.jetbrains.air.fork` and can
 * rewind to a named assistant turn. Matches `resolve_fork_point` in
 * `src-tauri/src/acp/fork.rs`.
 *
 * Other agents may still advertise `sessionCapabilities.fork` (Kimi, Qoder,
 * Grok, …) but they fork at the TAIL — the last round stays in the child
 * session, so an edit-and-resend of a LATER round would stack on top of the
 * old prompt instead of replacing it. Those later rounds stay hidden. The
 * first user message does not need a named fork: an empty session drops the
 * round for every agent.
 */
export const NAMED_FORK_AGENT_TYPES = [
  "claude_code",
  "codex",
  "deepseek",
] as const

export function agentSupportsNamedFork(agentType: AgentType): boolean {
  return (NAMED_FORK_AGENT_TYPES as readonly string[]).includes(agentType)
}

export interface LastRoundEditTarget {
  userTurn: MessageTurn
  /**
   * Assistant turn to named-fork at (the reply BEFORE the last user
   * message). `null` means rewind to an empty session — the ChatGPT
   * first-round case, where there is no earlier assistant to keep.
   */
  forkFromTurnId: string | null
  draft: PromptDraft
}

export interface TimelineTurnLike {
  turn: MessageTurn
  phase?: string
  isInFlightRound?: boolean
}

/**
 * The last completed/failed round can be overwritten when we can actually
 * drop it on the agent:
 *
 * - First user message (no earlier assistant): empty-session rewind, any
 *   agent. The following assistant, if any, is discarded.
 * - Later rounds: named-fork at the assistant BEFORE that user message.
 *   Tail-fork agents stay hidden — forking at the tail would keep the
 *   round and stack the edit on top of it.
 */
export function lastRoundEditTarget(
  timeline: TimelineTurnLike[],
  agentType: AgentType
): LastRoundEditTarget | null {
  let lastUserIdx = -1
  for (let i = timeline.length - 1; i >= 0; i--) {
    if (timeline[i].turn.role === "user") {
      lastUserIdx = i
      break
    }
  }
  if (lastUserIdx < 0) return null

  const lastUserEntry = timeline[lastUserIdx]
  if (
    lastUserEntry.phase === "optimistic" ||
    lastUserEntry.phase === "streaming"
  ) {
    return null
  }
  for (let i = lastUserIdx + 1; i < timeline.length; i++) {
    if (timeline[i].isInFlightRound || timeline[i].phase === "streaming") {
      return null
    }
  }

  const draft = userTurnToPromptDraft(lastUserEntry.turn)
  if (!draft) return null

  let prevAssistant: MessageTurn | null = null
  for (let i = lastUserIdx - 1; i >= 0; i--) {
    if (timeline[i].turn.role === "assistant") {
      prevAssistant = timeline[i].turn
      break
    }
  }
  if (!prevAssistant) {
    return {
      userTurn: lastUserEntry.turn,
      forkFromTurnId: null,
      draft,
    }
  }

  if (!agentSupportsNamedFork(agentType)) return null
  const forkFromTurnId = namedAssistantForkTurnId(prevAssistant, agentType)
  if (!forkFromTurnId) return null

  return {
    userTurn: lastUserEntry.turn,
    forkFromTurnId,
    draft,
  }
}

/** Id the backend's `resolve_fork_point` can look up. Live ids are unknown
 *  to the parser and would silently tail-fork. */
export function namedAssistantForkTurnId(
  turn: MessageTurn,
  agentType: AgentType
): string | null {
  if (turn.role !== "assistant") return null
  if (!agentSupportsNamedFork(agentType)) return null
  const id = turn.source_turn_id ?? turn.id
  if (id.startsWith("live-")) return null
  const text = assistantText(turn)
  if (agentType === "codex") {
    return text.trim().length > 0 ? id : null
  }
  if (!turn.agent_message_id && text.trim().length === 0) return null
  return id
}

export function userTurnToPromptDraft(turn: MessageTurn): PromptDraft | null {
  const blocks: PromptInputBlock[] = []
  for (const block of turn.blocks) {
    const converted = contentBlockToPromptInput(block)
    if (converted) blocks.push(converted)
  }
  if (blocks.length === 0) return null
  const displayText = blocks
    .filter(
      (block): block is Extract<PromptInputBlock, { type: "text" }> =>
        block.type === "text"
    )
    .map((block) => block.text)
    .join("\n")
  return { blocks, displayText }
}

function contentBlockToPromptInput(
  block: ContentBlock
): PromptInputBlock | null {
  switch (block.type) {
    case "text":
      return block.text.length > 0 ? { type: "text", text: block.text } : null
    case "image":
      if (!block.data || !block.mime_type) return null
      return {
        type: "image",
        data: block.data,
        mime_type: block.mime_type,
        uri: block.uri ?? null,
      }
    default:
      return null
  }
}

function assistantText(turn: MessageTurn): string {
  return turn.blocks
    .filter(
      (block): block is Extract<ContentBlock, { type: "text" }> =>
        block.type === "text"
    )
    .map((block) => block.text)
    .join("")
}
