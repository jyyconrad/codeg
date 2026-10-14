import { beforeEach, describe, expect, it, vi } from "vitest"

import { establishTerminalSession, terminalKill } from "@/lib/api"
import {
  claimSessionTerminal,
  noteSessionTerminalConversation,
  releaseSessionTerminal,
  sessionTerminalBinding,
} from "@/lib/session-terminal-session"
import {
  applyTerminalSessionEvent,
  releaseAndSettleSessionTerminal,
} from "@/lib/terminal-session-establish"
import type { AgentTerminalEvent } from "@/lib/types"
import {
  resetAppWorkspaceStore,
  useAppWorkspaceStore,
} from "@/stores/app-workspace-store"
import {
  resetTabStore,
  useTabStore,
  type TabItemInternal,
} from "@/stores/tab-store"

vi.mock("@/lib/api", () => ({
  establishTerminalSession: vi.fn(),
  terminalKill: vi.fn(async () => {}),
  listOpenedTabs: vi.fn(),
  saveOpenedTabs: vi.fn(),
  getFolderConversation: vi.fn(),
}))

const tab: TabItemInternal = {
  id: "tab-1",
  kind: "conversation",
  folderId: 4,
  conversationId: null,
  agentType: "claude_code",
  title: "新建会话",
  isPinned: true,
  workingDir: "/repo/codeg",
  terminalSession: true,
}

function hook(overrides: Partial<AgentTerminalEvent> = {}): AgentTerminalEvent {
  return {
    terminal_id: "term-1",
    agent_type: "claude_code",
    run_id: "run-1",
    event_id: "event-1",
    type: "session_started",
    native_event: "SessionStart",
    native_session_id: "sid-1",
    title: "Plan the release",
    received_at: "2026-09-30T00:00:00Z",
    ...overrides,
  }
}

beforeEach(() => {
  resetTabStore()
  resetAppWorkspaceStore()
  releaseSessionTerminal("tab-1")
  releaseSessionTerminal("canvas:card")
  vi.mocked(establishTerminalSession).mockReset()
  vi.mocked(terminalKill).mockClear()
  useTabStore.setState({ rawTabs: [tab], activeTabId: "tab-1", tabs: [tab] })
})

describe("applyTerminalSessionEvent", () => {
  it("binds the owning tab to the conversation the hook established", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    vi.mocked(establishTerminalSession).mockResolvedValue({
      conversationId: 9,
      folderId: 4,
      title: "Plan the release",
      folder: null,
    })

    applyTerminalSessionEvent(hook({ terminal_id: claimed.terminalId }))

    await vi.waitFor(() => {
      expect(useTabStore.getState().rawTabs[0].conversationId).toBe(9)
    })
    expect(useTabStore.getState().rawTabs[0]).toMatchObject({
      title: "Plan the release",
      terminalSession: true,
      workingDir: "/repo/codeg",
      folderId: 4,
    })
    expect(establishTerminalSession).toHaveBeenCalledWith({
      folderId: 4,
      agentType: "claude_code",
      externalId: "sid-1",
      preferredConversationId: null,
      title: "Plan the release",
      titleSeed: null,
      model: null,
      workingDir: "/repo/codeg",
      observation: "session_started",
      terminalId: claimed.terminalId,
      noticeBody: null,
    })
  })

  it("files the directory the terminal was started in, not a stale tab path", async () => {
    useTabStore.setState({
      rawTabs: [{ ...tab, workingDir: "/repo/old" }],
      activeTabId: "tab-1",
      tabs: [{ ...tab, workingDir: "/repo/old" }],
    })
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    vi.mocked(establishTerminalSession).mockResolvedValue({
      conversationId: 9,
      folderId: 4,
      title: "Plan the release",
      folder: null,
    })

    applyTerminalSessionEvent(hook({ terminal_id: claimed.terminalId }))

    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledWith(
        expect.objectContaining({ workingDir: "/repo/codeg", folderId: 4 })
      )
    })
  })

  it("ignores a canvas terminal and an event with no session id", async () => {
    const canvas = claimSessionTerminal("canvas:card", "grok", "/repo")
    applyTerminalSessionEvent(hook({ terminal_id: canvas.terminalId }))
    applyTerminalSessionEvent(
      hook({ native_session_id: undefined, terminal_id: "missing" })
    )
    await Promise.resolve()
    expect(establishTerminalSession).not.toHaveBeenCalled()
    expect(useTabStore.getState().rawTabs[0].conversationId).toBeNull()
  })

  it("serializes title updates for the same session", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    let release: (
      value: Awaited<ReturnType<typeof establishTerminalSession>>
    ) => void = () => {}
    vi.mocked(establishTerminalSession)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            release = resolve
          })
      )
      .mockResolvedValue({
        conversationId: 9,
        folderId: 4,
        title: "Better title",
      })

    applyTerminalSessionEvent(
      hook({ terminal_id: claimed.terminalId, event_id: "a", title: "First" })
    )
    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        event_id: "b",
        title: "Better title",
      })
    )
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    })

    release({ conversationId: 9, folderId: 4, title: "First" })
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(2)
    })
    await vi.waitFor(() => {
      expect(useTabStore.getState().rawTabs[0].title).toBe("Better title")
    })
    expect(useAppWorkspaceStore.getState().conversations).toEqual([])
  })

  it("waits for the first session id before establishing a second one", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    let release: (
      value: Awaited<ReturnType<typeof establishTerminalSession>>
    ) => void = () => {}
    vi.mocked(establishTerminalSession)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            release = resolve
          })
      )
      .mockResolvedValue({
        conversationId: 9,
        folderId: 4,
        title: "Second",
      })

    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        event_id: "a",
        native_session_id: "sid-1",
      })
    )
    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        event_id: "b",
        native_session_id: "sid-2",
        title: "Second",
      })
    )
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    })
    expect(establishTerminalSession).toHaveBeenCalledWith(
      expect.objectContaining({
        externalId: "sid-1",
        preferredConversationId: null,
      })
    )

    release({ conversationId: 9, folderId: 4, title: "First" })
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(2)
    })
    expect(establishTerminalSession).toHaveBeenLastCalledWith(
      expect.objectContaining({
        externalId: "sid-2",
        preferredConversationId: 9,
      })
    )
  })

  it("reports a dead process as process_exit and skips a missing row", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    useTabStore.setState({
      rawTabs: [{ ...tab, conversationId: 9 }],
      activeTabId: "tab-1",
      tabs: [{ ...tab, conversationId: 9 }],
    })
    vi.mocked(establishTerminalSession).mockResolvedValue({
      conversationId: 0,
      folderId: 0,
      title: null,
      absent: true,
    })

    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        type: "session_ended",
        native_event: "process_exit",
      })
    )

    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    })
    expect(establishTerminalSession).toHaveBeenCalledWith(
      expect.objectContaining({
        observation: "process_exit",
        preferredConversationId: 9,
        externalId: "sid-1",
      })
    )
    expect(useTabStore.getState().rawTabs[0].conversationId).toBe(9)
    expect(sessionTerminalBinding("tab-1")?.conversationId).toBeNull()
  })

  it("settles the stored row when the tab is already gone", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    noteSessionTerminalConversation("tab-1", claimed.terminalId, 9)
    useTabStore.setState({ rawTabs: [], activeTabId: null, tabs: [] })

    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        type: "session_ended",
        native_event: "process_exit",
      })
    )

    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    })
    expect(establishTerminalSession).toHaveBeenCalledWith(
      expect.objectContaining({
        folderId: 0,
        externalId: "process-exit",
        preferredConversationId: 9,
        observation: "process_exit",
        terminalId: claimed.terminalId,
      })
    )
  })

  it("does not open a row for a hook that arrives after the tab is gone", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    noteSessionTerminalConversation("tab-1", claimed.terminalId, 9)
    useTabStore.setState({ rawTabs: [], activeTabId: null, tabs: [] })

    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        type: "turn_started",
        native_event: "UserPromptSubmit",
      })
    )
    await Promise.resolve()

    expect(establishTerminalSession).not.toHaveBeenCalled()
  })

  it("settles the bound row on release and drops a hook that arrives after", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    noteSessionTerminalConversation("tab-1", claimed.terminalId, 9)
    vi.mocked(establishTerminalSession).mockResolvedValue({
      conversationId: 9,
      folderId: 4,
      title: "Plan the release",
    })

    releaseAndSettleSessionTerminal("tab-1")
    applyTerminalSessionEvent(
      hook({
        terminal_id: claimed.terminalId,
        type: "turn_started",
        native_event: "UserPromptSubmit",
        event_id: "late",
      })
    )

    await vi.waitFor(() => {
      expect(terminalKill).toHaveBeenCalledWith(claimed.terminalId)
    })
    expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    expect(establishTerminalSession).toHaveBeenCalledWith(
      expect.objectContaining({
        folderId: 0,
        externalId: "process-exit",
        preferredConversationId: 9,
        observation: "process_exit",
      })
    )
    expect(useTabStore.getState().rawTabs[0].conversationId).toBeNull()
  })

  it("waits for an in-flight bind before settling the released terminal", async () => {
    const claimed = claimSessionTerminal("tab-1", "claude_code", "/repo/codeg")
    let release: (
      value: Awaited<ReturnType<typeof establishTerminalSession>>
    ) => void = () => {}
    vi.mocked(establishTerminalSession)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            release = resolve
          })
      )
      .mockResolvedValue({
        conversationId: 9,
        folderId: 4,
        title: "Plan the release",
      })

    applyTerminalSessionEvent(hook({ terminal_id: claimed.terminalId }))
    releaseAndSettleSessionTerminal("tab-1")
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(1)
    })
    expect(terminalKill).not.toHaveBeenCalled()

    release({ conversationId: 9, folderId: 4, title: "Plan the release" })
    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledTimes(2)
    })
    expect(establishTerminalSession).toHaveBeenLastCalledWith(
      expect.objectContaining({
        observation: "process_exit",
        preferredConversationId: 9,
        externalId: "process-exit",
      })
    )
    await vi.waitFor(() => {
      expect(terminalKill).toHaveBeenCalledWith(claimed.terminalId)
    })
  })
})
