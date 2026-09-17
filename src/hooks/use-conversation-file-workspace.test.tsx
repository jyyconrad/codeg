import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { act, renderHook } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import type { FileWorkspaceTab } from "@/contexts/workspace-context"
import { useConversationScopedFileWorkspace } from "@/hooks/use-conversation-file-workspace"
import type { FileWorkspaceSnapshot } from "@/lib/conversation-file-workspace"

const workspace = vi.hoisted(() => {
  const state: FileWorkspaceSnapshot = {
    fileTabs: [],
    activeFileTabId: null,
    previewFileTabIds: [],
    filesMaximized: false,
    pendingFileReveal: null,
  }
  return {
    state,
    captureFileWorkspace: vi.fn(
      (): FileWorkspaceSnapshot => ({
        fileTabs: state.fileTabs,
        activeFileTabId: state.activeFileTabId,
        previewFileTabIds: [...state.previewFileTabIds],
        filesMaximized: state.filesMaximized,
        pendingFileReveal: state.pendingFileReveal,
      })
    ),
    replaceFileWorkspace: vi.fn((snapshot: FileWorkspaceSnapshot) => {
      state.fileTabs = snapshot.fileTabs
      state.activeFileTabId = snapshot.activeFileTabId
      state.previewFileTabIds = [...snapshot.previewFileTabIds]
      state.pendingFileReveal = snapshot.pendingFileReveal
      state.filesMaximized = snapshot.filesMaximized
    }),
    reset() {
      state.fileTabs = []
      state.activeFileTabId = null
      state.previewFileTabIds = []
      state.pendingFileReveal = null
      state.filesMaximized = false
      workspace.captureFileWorkspace.mockClear()
      workspace.replaceFileWorkspace.mockClear()
    },
  }
})

vi.mock("@/contexts/workspace-context", () => ({
  useWorkspaceActions: () => ({
    captureFileWorkspace: workspace.captureFileWorkspace,
    replaceFileWorkspace: workspace.replaceFileWorkspace,
  }),
}))

function tab(path: string): FileWorkspaceTab {
  return {
    id: `file:${path}`,
    kind: "file",
    folderId: null,
    title: path,
    description: null,
    path,
    language: "plaintext",
    content: "",
    loading: false,
  }
}

describe("useConversationScopedFileWorkspace", () => {
  it("captures the file column through the stable actions slice", () => {
    // Subscribing to fileTabs from the conversation panel would re-render
    // every keep-alive transcript on each editor keystroke.
    const source = readFileSync(
      resolve(process.cwd(), "src/hooks/use-conversation-file-workspace.ts"),
      "utf8"
    )
    expect(source).toContain("captureFileWorkspace")
    expect(source).not.toContain("useWorkspaceFileTabs")
  })

  it("parks conversation A's preview and restores it when switching back", () => {
    workspace.reset()
    const { rerender } = renderHook(
      ({ active, live }: { active: string; live: string[] }) =>
        useConversationScopedFileWorkspace(active, live),
      { initialProps: { active: "conv-a", live: ["conv-a", "conv-b"] } }
    )

    workspace.state.fileTabs = [tab("/repo/a.ts")]
    workspace.state.activeFileTabId = "file:/repo/a.ts"

    act(() => {
      rerender({ active: "conv-b", live: ["conv-a", "conv-b"] })
    })
    expect(workspace.replaceFileWorkspace).toHaveBeenCalledTimes(1)
    expect(workspace.replaceFileWorkspace.mock.calls[0][0].fileTabs).toEqual([])

    workspace.state.fileTabs = [tab("/repo/b.ts")]
    workspace.state.activeFileTabId = "file:/repo/b.ts"

    act(() => {
      rerender({ active: "conv-a", live: ["conv-a", "conv-b"] })
    })
    const restored =
      workspace.replaceFileWorkspace.mock.calls[
        workspace.replaceFileWorkspace.mock.calls.length - 1
      ][0]
    expect(restored.fileTabs.map((item) => item.path)).toEqual(["/repo/a.ts"])
  })

  it("does not wipe files that were open before the first conversation tab", () => {
    workspace.reset()
    workspace.state.fileTabs = [tab("/repo/a.ts")]
    workspace.state.activeFileTabId = "file:/repo/a.ts"
    const { rerender } = renderHook(
      ({ active, live }: { active: string | null; live: string[] }) =>
        useConversationScopedFileWorkspace(active, live),
      { initialProps: { active: null, live: [] as string[] } }
    )

    act(() => {
      rerender({ active: "conv-a", live: ["conv-a"] })
    })
    expect(workspace.replaceFileWorkspace).not.toHaveBeenCalled()
  })
})
