import type { AgentType } from "@/lib/types"
import { randomUUID } from "@/lib/utils"

/**
 * The PTY a draft session owns while its agent is in terminal launch mode.
 *
 * The id lives outside the React view so dragging a tab into another split
 * group (which remounts the view) reattaches to the same process. Closing the
 * session releases the id; the view then kills that process.
 */
interface SessionTerminalRecord {
  terminalId: string
  agentType: AgentType
  workingDir: string
  /** Conversation the hook already bound. Null until the first establish. */
  conversationId: number | null
}

const sessions = new Map<string, SessionTerminalRecord>()

/**
 * Directory a terminal CLI is started in.
 * The folder row's live path wins over a draft's stored path, so a moved
 * folder or a stale draft cannot start the CLI in the previous project.
 * A chat scratch dir has no folder path yet and stays as stored.
 */
export function terminalLaunchDirectory(
  folderPath: string | null | undefined,
  storedWorkingDir: string | null | undefined
): string | undefined {
  const folder = folderPath?.trim()
  if (folder) return folder
  const stored = storedWorkingDir?.trim()
  return stored || undefined
}

export function claimSessionTerminal(
  ownerKey: string,
  agentType: AgentType,
  workingDir: string,
  preferredTerminalId?: string | null
): {
  terminalId: string
  replacedId: string | null
  replacedConversationId: number | null
} {
  const existing = sessions.get(ownerKey)
  if (
    existing &&
    existing.agentType === agentType &&
    existing.workingDir === workingDir
  ) {
    return {
      terminalId: existing.terminalId,
      replacedId: null,
      replacedConversationId: null,
    }
  }
  const preferred = preferredTerminalId?.trim()
  const terminalId = preferred ? preferred : randomUUID()
  sessions.set(ownerKey, {
    terminalId,
    agentType,
    workingDir,
    conversationId: null,
  })
  return {
    terminalId,
    replacedId: existing?.terminalId ?? null,
    replacedConversationId: existing?.conversationId ?? null,
  }
}

/** Remember the row a live PTY established. A replaced PTY does not inherit it. */
export function noteSessionTerminalConversation(
  ownerKey: string,
  terminalId: string,
  conversationId: number
): void {
  const existing = sessions.get(ownerKey)
  if (!existing || existing.terminalId !== terminalId || conversationId <= 0) {
    return
  }
  existing.conversationId = conversationId
}

export function sessionTerminalBinding(ownerKey: string): {
  terminalId: string
  agentType: AgentType
  workingDir: string
  conversationId: number | null
} | null {
  const existing = sessions.get(ownerKey)
  if (!existing) return null
  return {
    terminalId: existing.terminalId,
    agentType: existing.agentType,
    workingDir: existing.workingDir,
    conversationId: existing.conversationId,
  }
}

/** Forget the session's terminal. Returns the id the caller should kill. */
export function releaseSessionTerminal(ownerKey: string): string | null {
  const existing = sessions.get(ownerKey)
  if (!existing) return null
  sessions.delete(ownerKey)
  return existing.terminalId
}

/** Tab or canvas key that currently owns this PTY, if one does. */
export function sessionTerminalOwnerByTerminal(
  terminalId: string
): string | null {
  for (const [ownerKey, record] of sessions) {
    if (record.terminalId === terminalId) return ownerKey
  }
  return null
}
