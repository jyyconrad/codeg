import type { AgentTerminalEvent } from "@/lib/types"

/** Identity a terminal hook can turn into a conversation row. */
export interface TerminalSessionClaim {
  externalId: string
  title?: string
  titleSeed?: string
  model?: string
}

function nonempty(value: string | null | undefined): string | undefined {
  const text = value?.trim()
  return text ? text : undefined
}

/**
 * Pull the session id and title fields off a terminal hook event.
 * Turn events count: Codex reports the session on the first prompt, not on
 * SessionStart. A blank session id establishes nothing.
 */
export function claimFromTerminalEvent(
  event: AgentTerminalEvent
): TerminalSessionClaim | null {
  const externalId = nonempty(event.native_session_id)
  if (!externalId) return null
  if (
    event.type !== "session_started" &&
    event.type !== "turn_started" &&
    event.type !== "turn_stopped" &&
    event.type !== "session_ended"
  ) {
    return null
  }
  return {
    externalId,
    title: nonempty(event.title),
    titleSeed: nonempty(event.title_seed),
    model: nonempty(event.model),
  }
}
