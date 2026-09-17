"use client"

import { useEffect, useRef } from "react"

import { useWorkspaceActions } from "@/contexts/workspace-context"
import {
  dropClosedConversationSnapshots,
  switchConversationFileWorkspace,
  type FileWorkspaceSnapshot,
} from "@/lib/conversation-file-workspace"

/**
 * Parks the file column with the conversation that opened it, and restores
 * the incoming conversation's preview (or hides the column) on tab switch.
 *
 * Conversation tabs stay mounted when backgrounded; the file column does not
 * — it is a single workspace-global pane. This hook is the mapping between
 * the two, so A can keep attachment A while B keeps attachment B.
 *
 * Reads the parked snapshot through `captureFileWorkspace` (ref) rather than
 * subscribing to `fileTabs`, so the conversation panel does not re-render on
 * every keystroke in the editor.
 */
export function useConversationScopedFileWorkspace(
  activeTabId: string | null,
  liveTabIds: readonly string[]
) {
  const { captureFileWorkspace, replaceFileWorkspace } = useWorkspaceActions()

  const storeRef = useRef(new Map<string, FileWorkspaceSnapshot>())
  const prevIdRef = useRef(activeTabId)
  const liveKey = liveTabIds.join("\0")

  useEffect(() => {
    const fromId = prevIdRef.current
    const toId = activeTabId
    const current = captureFileWorkspace()
    let store = storeRef.current
    let next = current

    if (fromId !== toId) {
      const switched = switchConversationFileWorkspace(
        store,
        current,
        fromId,
        toId
      )
      store = switched.store
      next = switched.next
      prevIdRef.current = toId
    }

    const live = new Set(liveKey.length === 0 ? [] : liveKey.split("\0"))
    const dropped = dropClosedConversationSnapshots(store, next, live)
    storeRef.current = dropped.store

    if (dropped.next !== current) {
      replaceFileWorkspace(dropped.next)
    }
  }, [activeTabId, liveKey, captureFileWorkspace, replaceFileWorkspace])
}
