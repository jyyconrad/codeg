import type { FileWorkspaceTab } from "@/contexts/workspace-context"

/**
 * The file column is workspace-global, but a preview opened from a transcript
 * belongs to that conversation. Switching tabs parks the outgoing column here
 * and restores the incoming one (empty = the column hides).
 */
export interface FileWorkspaceSnapshot {
  fileTabs: FileWorkspaceTab[]
  activeFileTabId: string | null
  previewFileTabIds: string[]
  filesMaximized: boolean
  pendingFileReveal: {
    requestId: number
    path: string
    line: number
  } | null
}

export function emptyFileWorkspaceSnapshot(): FileWorkspaceSnapshot {
  return {
    fileTabs: [],
    activeFileTabId: null,
    previewFileTabIds: [],
    filesMaximized: false,
    pendingFileReveal: null,
  }
}

export function switchConversationFileWorkspace(
  store: Map<string, FileWorkspaceSnapshot>,
  current: FileWorkspaceSnapshot,
  fromId: string | null,
  toId: string | null
): {
  store: Map<string, FileWorkspaceSnapshot>
  next: FileWorkspaceSnapshot
} {
  if (fromId === toId) {
    return { store, next: current }
  }
  const nextStore = new Map(store)
  if (fromId != null) {
    nextStore.set(fromId, current)
  }
  if (toId == null) {
    return { store: nextStore, next: emptyFileWorkspaceSnapshot() }
  }
  // Files opened before any conversation tab existed (the file tree, a
  // canvas card) are not owned yet — keep them on the first session rather
  // than wiping the column as if the user had switched away from one.
  if (fromId == null) {
    return { store: nextStore, next: current }
  }
  return {
    store: nextStore,
    next: nextStore.get(toId) ?? emptyFileWorkspaceSnapshot(),
  }
}

/**
 * A closed conversation's parked files would otherwise vanish with the tab.
 * Dirty tabs are rescued into the current workspace so unsaved edits survive;
 * clean tabs are on disk and can go.
 */
export function dropClosedConversationSnapshots(
  store: Map<string, FileWorkspaceSnapshot>,
  current: FileWorkspaceSnapshot,
  liveTabIds: ReadonlySet<string>
): {
  store: Map<string, FileWorkspaceSnapshot>
  next: FileWorkspaceSnapshot
} {
  const nextStore = new Map<string, FileWorkspaceSnapshot>()
  const rescue: FileWorkspaceTab[] = []
  for (const [id, snapshot] of store) {
    if (liveTabIds.has(id)) {
      nextStore.set(id, snapshot)
      continue
    }
    for (const tab of snapshot.fileTabs) {
      if (tab.kind === "file" && tab.isDirty) rescue.push(tab)
    }
  }
  if (rescue.length === 0) {
    return { store: nextStore, next: current }
  }
  const seen = new Set(current.fileTabs.map((tab) => tab.id))
  const extra = rescue.filter((tab) => !seen.has(tab.id))
  if (extra.length === 0) {
    return { store: nextStore, next: current }
  }
  const fileTabs = [...current.fileTabs, ...extra]
  return {
    store: nextStore,
    next: {
      ...current,
      fileTabs,
      activeFileTabId: current.activeFileTabId ?? extra[0].id,
    },
  }
}
