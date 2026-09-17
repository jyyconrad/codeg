/**
 * Sidebar "needs a look" markers for conversations that finished or failed
 * while the user was elsewhere. A click (or having the row selected) records
 * the current status epoch so the unread pip goes away until the conversation
 * finishes or fails again. Restarting a failed run (`in_progress`) is not an
 * attention state — the running spinner replaces both the X and the red pip.
 */

export const CONVERSATION_ATTENTION_STORAGE_KEY =
  "codeg.conversation-attention.seen.v1"

export type ConversationAttentionKind = "completed" | "failed"

type SeenMap = Record<string, string>

let seen: SeenMap = {}
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of listeners) listener()
}

function persist(): void {
  if (typeof window === "undefined") return
  try {
    window.localStorage.setItem(
      CONVERSATION_ATTENTION_STORAGE_KEY,
      JSON.stringify(seen)
    )
  } catch {
    /* quota / private mode */
  }
}

export function subscribeConversationAttention(
  listener: () => void
): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function getConversationAttentionSeenEpoch(
  id: number
): string | undefined {
  return seen[String(id)]
}

export function conversationSeenEpoch(
  status: string,
  updatedAt: string
): string {
  return `${status}:${updatedAt}`
}

export function conversationAttentionKind(
  status: string
): ConversationAttentionKind | null {
  if (status === "cancelled") return "failed"
  if (status === "pending_review" || status === "completed") return "completed"
  return null
}

export function conversationShowsAttention(
  status: string,
  id: number,
  updatedAt: string,
  isSelected: boolean
): ConversationAttentionKind | null {
  if (isSelected) return null
  const kind = conversationAttentionKind(status)
  if (!kind) return null
  const epoch = conversationSeenEpoch(status, updatedAt)
  if (seen[String(id)] === epoch) return null
  return kind
}

export function markConversationAttentionSeen(
  id: number,
  epoch: string,
  options?: { notify?: boolean }
): void {
  const key = String(id)
  if (seen[key] === epoch) return
  seen = { ...seen, [key]: epoch }
  persist()
  if (options?.notify === false) return
  emit()
}

export function hydrateConversationAttention(): void {
  if (typeof window === "undefined") return
  try {
    const raw = window.localStorage.getItem(CONVERSATION_ATTENTION_STORAGE_KEY)
    if (!raw) {
      seen = {}
      emit()
      return
    }
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      seen = {}
      emit()
      return
    }
    const next: SeenMap = {}
    for (const [key, value] of Object.entries(
      parsed as Record<string, unknown>
    )) {
      if (typeof value === "string") next[key] = value
    }
    seen = next
    emit()
  } catch {
    seen = {}
    emit()
  }
}

export function resetConversationAttention(): void {
  seen = {}
  if (typeof window !== "undefined") {
    try {
      window.localStorage.removeItem(CONVERSATION_ATTENTION_STORAGE_KEY)
    } catch {
      /* ignore */
    }
  }
  emit()
}

if (typeof window !== "undefined") {
  hydrateConversationAttention()
}
