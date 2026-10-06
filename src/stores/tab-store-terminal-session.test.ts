import { beforeEach, describe, expect, it, vi } from "vitest"

import type { FolderDetail } from "@/lib/types"
import {
  resetAppWorkspaceStore,
  useAppWorkspaceStore,
} from "./app-workspace-store"
import { resetTabStore, useTabStore, type TabItemInternal } from "./tab-store"

vi.mock("@/lib/api", () => ({
  listOpenedTabs: vi.fn(),
  saveOpenedTabs: vi.fn(),
  getFolderConversation: vi.fn(),
}))

vi.mock("@/lib/platform", () => ({
  subscribe: vi.fn(),
  onTransportReconnect: vi.fn(),
}))

const folder = {
  id: 1,
  name: "repo",
  path: "/repo",
  kind: "regular",
} as unknown as FolderDetail

function draft(id: string, terminalSession = false): TabItemInternal {
  return {
    id,
    kind: "conversation",
    folderId: 1,
    conversationId: null,
    agentType: "grok",
    title: "新建会话",
    isPinned: true,
    workingDir: "/repo",
    ...(terminalSession ? { terminalSession: true } : {}),
  }
}

beforeEach(() => {
  resetTabStore()
  resetAppWorkspaceStore()
  useAppWorkspaceStore.setState({ folders: [folder], allFolders: [folder] })
})

describe("terminal session drafts", () => {
  it("opens another draft instead of reusing a running terminal CLI", () => {
    useTabStore.setState({
      rawTabs: [draft("term-tab", true)],
      activeTabId: "term-tab",
    })

    const opened = useTabStore
      .getState()
      .openNewConversationTab(1, "/repo", { forceAgent: "grok" })

    expect(opened.tabId).not.toBe("term-tab")
    const tabs = useTabStore.getState().rawTabs
    expect(tabs).toHaveLength(2)
    expect(tabs[0]).toMatchObject({
      id: "term-tab",
      terminalSession: true,
      agentType: "grok",
      conversationId: null,
    })
    expect(useTabStore.getState().activeTabId).toBe(opened.tabId)
  })

  it("still reuses an ordinary empty draft", () => {
    useTabStore.setState({
      rawTabs: [draft("plain")],
      activeTabId: "plain",
    })

    const opened = useTabStore
      .getState()
      .openNewConversationTab(1, "/repo", { forceAgent: "grok" })

    expect(opened.tabId).toBe("plain")
    expect(useTabStore.getState().rawTabs).toHaveLength(1)
    expect(useTabStore.getState().activeTabId).toBe("plain")
  })

  it("does not turn a terminal draft into a chat draft", () => {
    useTabStore.setState({
      rawTabs: [draft("term-tab", true)],
      activeTabId: "term-tab",
    })

    const opened = useTabStore
      .getState()
      .openChatModeTab({ forceAgent: "grok" })

    expect(opened.tabId).not.toBe("term-tab")
    const occupied = useTabStore
      .getState()
      .rawTabs.find((tab) => tab.id === "term-tab")
    expect(occupied).toMatchObject({
      terminalSession: true,
      folderId: 1,
    })
    expect(occupied?.isChat).toBeUndefined()
    expect(
      useTabStore.getState().rawTabs.find((tab) => tab.id === opened.tabId)
    ).toMatchObject({ isChat: true, conversationId: null })
  })

  it("keeps the terminal surface after the conversation row is bound", () => {
    useTabStore.setState({
      rawTabs: [draft("term-tab", true)],
      activeTabId: "term-tab",
    })
    useTabStore
      .getState()
      .bindConversationTab("term-tab", 42, "grok", "Plan the release")

    const bound = useTabStore.getState().rawTabs[0]
    expect(bound).toMatchObject({
      conversationId: 42,
      terminalSession: true,
      title: "Plan the release",
    })

    useTabStore.getState().clearTerminalSession("term-tab")
    expect(useTabStore.getState().rawTabs[0].terminalSession).toBe(true)

    const opened = useTabStore
      .getState()
      .openNewConversationTab(1, "/repo", { forceAgent: "grok" })
    expect(opened.tabId).not.toBe("term-tab")
  })

  it("releases an unbound terminal draft when the process exits", () => {
    useTabStore.setState({
      rawTabs: [draft("term-tab", true)],
      activeTabId: "term-tab",
    })

    useTabStore.getState().clearTerminalSession("term-tab")

    const opened = useTabStore
      .getState()
      .openNewConversationTab(1, "/repo", { forceAgent: "grok" })
    expect(opened.tabId).toBe("term-tab")
  })
})
