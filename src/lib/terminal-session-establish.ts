import { establishTerminalSession, terminalKill } from "@/lib/api"
import { formatConversationTitle } from "@/lib/conversation-title"
import {
  noteSessionTerminalConversation,
  releaseSessionTerminal,
  sessionTerminalBinding,
  sessionTerminalOwnerByTerminal,
} from "@/lib/session-terminal-session"
import {
  claimFromTerminalEvent,
  type TerminalSessionClaim,
} from "@/lib/terminal-session-claim"
import type { AgentTerminalEvent, AgentType } from "@/lib/types"
import { useAppWorkspaceStore } from "@/stores/app-workspace-store"
import { useTabStore } from "@/stores/tab-store"

const inflight = new Map<string, Promise<void>>()

function observationName(event: AgentTerminalEvent): string {
  return event.native_event === "process_exit" ? "process_exit" : event.type
}

function enqueue(ownerKey: string, work: () => Promise<void>): void {
  // One chain per terminal, not per session id. A second id that arrives
  // before the first bind lands would otherwise create another untitled row.
  // Releasing the PTY joins the same chain, so a hook still in flight can
  // record the row before that release cancels a running turn.
  const previous = inflight.get(ownerKey) ?? Promise.resolve()
  const next = previous
    .catch(() => {})
    .then(work)
    .catch((err: unknown) => {
      console.error("[terminal-session] establish:", err)
    })
  inflight.set(ownerKey, next)
  void next.finally(() => {
    if (inflight.get(ownerKey) === next) inflight.delete(ownerKey)
  })
}

/**
 * Turn a terminal hook into a conversation row and bind the tab that owns the
 * PTY. Canvas terminals have no tab, so they are ignored. Failures are logged;
 * the CLI keeps running either way.
 */
export function applyTerminalSessionEvent(event: AgentTerminalEvent): void {
  const claim = claimFromTerminalEvent(event)
  if (!claim) return
  const ownerKey = sessionTerminalOwnerByTerminal(event.terminal_id)
  if (!ownerKey) return
  const tab = useTabStore
    .getState()
    .rawTabs.find((item) => item.id === ownerKey)
  if (!tab) {
    // The tab is already gone, but the PTY record can still name the row.
    // Settle that row; a normal hook must not create another one.
    if (event.native_event !== "process_exit") return
    const conversationId = sessionTerminalBinding(ownerKey)?.conversationId
    if (conversationId == null || conversationId <= 0) return
    enqueue(ownerKey, () =>
      settleTerminalProcessExit(
        event.agent_type,
        conversationId,
        event.terminal_id
      )
    )
    return
  }
  enqueue(ownerKey, () => establishOwnedSession(ownerKey, claim, event))
}

/**
 * Cancel a still-running turn on a conversation this terminal already bound.
 * Sends `process_exit`, which never inserts a row. An idle row is left idle.
 */
export async function settleTerminalProcessExit(
  agentType: AgentType,
  conversationId: number,
  terminalId: string
): Promise<void> {
  if (conversationId <= 0) return
  try {
    await establishTerminalSession({
      folderId: 0,
      agentType,
      externalId: "process-exit",
      preferredConversationId: conversationId,
      title: null,
      titleSeed: null,
      model: null,
      workingDir: null,
      observation: "process_exit",
      terminalId,
      noticeBody: null,
    })
  } catch (err: unknown) {
    console.error("[terminal-session] settle:", err)
  }
}

/**
 * Drop the PTY record, cancel a still-running turn, then kill the process.
 * A split-group drag must not call this. The remount reattaches to the same id.
 */
export function releaseAndSettleSessionTerminal(ownerKey: string): void {
  const binding = sessionTerminalBinding(ownerKey)
  if (!binding) return
  const { terminalId, agentType } = binding
  enqueue(ownerKey, async () => {
    const current = sessionTerminalBinding(ownerKey)
    if (!current || current.terminalId !== terminalId) return
    const conversationId = current.conversationId
    releaseSessionTerminal(ownerKey)
    if (conversationId != null && conversationId > 0) {
      await settleTerminalProcessExit(agentType, conversationId, terminalId)
    }
    await terminalKill(terminalId).catch(() => {})
  })
}

async function establishOwnedSession(
  ownerKey: string,
  claim: TerminalSessionClaim,
  event: AgentTerminalEvent
): Promise<void> {
  const binding = sessionTerminalBinding(ownerKey)
  if (!binding || binding.terminalId !== event.terminal_id) return
  const tab = useTabStore
    .getState()
    .rawTabs.find((item) => item.id === ownerKey)
  if (!tab) {
    if (event.native_event !== "process_exit") return
    if (binding.conversationId == null || binding.conversationId <= 0) return
    await settleTerminalProcessExit(
      event.agent_type,
      binding.conversationId,
      event.terminal_id
    )
    return
  }
  const result = await establishTerminalSession({
    folderId: tab.folderId,
    agentType: tab.agentType,
    externalId: claim.externalId,
    preferredConversationId: tab.conversationId ?? binding.conversationId,
    title: claim.title ?? null,
    titleSeed: claim.titleSeed ?? null,
    model: claim.model ?? null,
    workingDir: tab.workingDir ?? null,
    observation: observationName(event),
    terminalId: event.terminal_id,
    noticeBody: event.notice_body ?? null,
  })
  if (result.absent || result.conversationId <= 0) return
  const still = sessionTerminalBinding(ownerKey)
  if (!still || still.terminalId !== event.terminal_id) return
  noteSessionTerminalConversation(
    ownerKey,
    event.terminal_id,
    result.conversationId
  )
  const fresh = useTabStore
    .getState()
    .rawTabs.find((item) => item.id === ownerKey)
  if (!fresh) return
  if (result.folder) {
    useAppWorkspaceStore.getState().upsertFolder(result.folder)
  }
  const displayTitle = result.title?.trim() || fresh.title
  const formatted = formatConversationTitle(displayTitle) || fresh.title
  if (
    fresh.conversationId !== result.conversationId ||
    fresh.folderId !== result.folderId ||
    fresh.title !== formatted
  ) {
    useTabStore
      .getState()
      .bindConversationTab(
        ownerKey,
        result.conversationId,
        fresh.agentType,
        displayTitle,
        fresh.runtimeConversationId,
        result.folderId,
        fresh.workingDir
      )
  }
  useTabStore.getState().markTerminalSession(ownerKey)
}
