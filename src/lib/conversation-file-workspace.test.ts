import { describe, expect, it } from "vitest"

import type { FileWorkspaceTab } from "@/contexts/workspace-context"
import {
  dropClosedConversationSnapshots,
  emptyFileWorkspaceSnapshot,
  switchConversationFileWorkspace,
  type FileWorkspaceSnapshot,
} from "@/lib/conversation-file-workspace"

function tab(id: string, dirty = false): FileWorkspaceTab {
  return {
    id,
    kind: "file",
    folderId: null,
    title: id,
    description: null,
    path: `/${id}`,
    language: "plaintext",
    content: dirty ? "edited" : "",
    loading: false,
    isDirty: dirty,
  }
}

function snap(tabs: FileWorkspaceTab[]): FileWorkspaceSnapshot {
  return {
    fileTabs: tabs,
    activeFileTabId: tabs[0]?.id ?? null,
    previewFileTabIds: tabs.map((item) => item.id),
    filesMaximized: false,
    pendingFileReveal: null,
  }
}

describe("switchConversationFileWorkspace", () => {
  it("parks the outgoing conversation's files and restores the incoming one", () => {
    const a = snap([tab("file-a")])
    const b = snap([tab("file-b")])
    const store = new Map<string, FileWorkspaceSnapshot>([["conv-b", b]])

    const switched = switchConversationFileWorkspace(
      store,
      a,
      "conv-a",
      "conv-b"
    )

    expect(switched.next.fileTabs.map((item) => item.id)).toEqual(["file-b"])
    expect(
      switched.store.get("conv-a")?.fileTabs.map((item) => item.id)
    ).toEqual(["file-a"])
  })

  it("hides the file workspace when the incoming conversation has no preview", () => {
    const a = snap([tab("file-a")])
    const switched = switchConversationFileWorkspace(
      new Map(),
      a,
      "conv-a",
      "conv-b"
    )

    expect(switched.next).toEqual(emptyFileWorkspaceSnapshot())
    expect(switched.store.get("conv-a")?.fileTabs).toHaveLength(1)
  })

  it("keeps already-open files when the first conversation tab appears", () => {
    const current = snap([tab("file-a")])
    const switched = switchConversationFileWorkspace(
      new Map(),
      current,
      null,
      "conv-a"
    )
    expect(switched.next).toBe(current)
  })

  it("is a no-op when the conversation id does not change", () => {
    const a = snap([tab("file-a")])
    const store = new Map<string, FileWorkspaceSnapshot>()
    const switched = switchConversationFileWorkspace(
      store,
      a,
      "conv-a",
      "conv-a"
    )
    expect(switched.next).toBe(a)
    expect(switched.store).toBe(store)
  })
})

describe("dropClosedConversationSnapshots", () => {
  it("rescues dirty files from a closed conversation so unsaved edits are not dropped", () => {
    const store = new Map<string, FileWorkspaceSnapshot>([
      ["gone", snap([tab("dirty", true), tab("clean")])],
      ["live", snap([tab("kept")])],
    ])
    const current = emptyFileWorkspaceSnapshot()
    const dropped = dropClosedConversationSnapshots(
      store,
      current,
      new Set(["live"])
    )

    expect(dropped.store.has("gone")).toBe(false)
    expect(dropped.next.fileTabs.map((item) => item.id)).toEqual(["dirty"])
  })

  it("does not duplicate a dirty tab already in the current workspace", () => {
    const dirty = tab("dirty", true)
    const store = new Map<string, FileWorkspaceSnapshot>([
      ["gone", snap([dirty])],
    ])
    const dropped = dropClosedConversationSnapshots(
      store,
      snap([dirty]),
      new Set()
    )
    expect(dropped.next.fileTabs).toHaveLength(1)
  })
})
