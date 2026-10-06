import { render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"

vi.mock("next-intl", () => ({
  useTranslations: () => (key: string, values?: { agent?: string }) =>
    values?.agent ? `${key}:${values.agent}` : key,
}))

const { markTerminalSession } = vi.hoisted(() => ({
  markTerminalSession: vi.fn(),
}))

vi.mock("@/stores/tab-store", () => ({
  useTabStore: {
    getState: () => ({
      markTerminalSession,
      clearTerminalSession: vi.fn(),
    }),
  },
}))

vi.mock("@/components/terminal/terminal-view", () => ({
  TerminalView: ({
    agentLaunch,
    terminalId,
    isActive,
    isVisible,
    keepVisible,
  }: {
    agentLaunch?: { prompt: string }
    terminalId: string
    isActive?: boolean
    isVisible?: boolean
    keepVisible?: boolean
  }) => (
    <div
      data-testid="session-terminal-view"
      data-prompt={agentLaunch?.prompt ?? "missing"}
      data-id={terminalId}
      data-active={String(isActive)}
      data-visible={String(isVisible)}
      data-keep={String(keepVisible)}
    />
  ),
}))

vi.mock("@/lib/api", () => ({
  terminalKill: vi.fn(async () => {}),
  openSettingsWindow: vi.fn(async () => {}),
  establishTerminalSession: vi.fn(async () => ({
    conversationId: 9,
    folderId: 4,
    title: null,
  })),
}))

import { establishTerminalSession, terminalKill } from "@/lib/api"
import {
  noteSessionTerminalConversation,
  releaseSessionTerminal,
} from "@/lib/session-terminal-session"
import { SessionTerminalInput } from "./session-terminal-input"

afterEach(() => {
  releaseSessionTerminal("tab-a")
  vi.mocked(terminalKill).mockClear()
  vi.mocked(establishTerminalSession).mockClear()
  markTerminalSession.mockClear()
})

describe("SessionTerminalInput", () => {
  it("starts the agent CLI with an empty prompt and no host chrome", async () => {
    render(
      <SessionTerminalInput
        ownerKey="tab-a"
        agentType="codex"
        agentName="Codex"
        workingDir="/repo/codeg"
        isActive
      />
    )

    const view = await screen.findByTestId("session-terminal-view")
    expect(view.getAttribute("data-prompt")).toBe("")
    expect(view.getAttribute("data-active")).toBe("true")
    expect(view.getAttribute("data-visible")).toBe("true")
    expect(view.getAttribute("data-keep")).toBe("true")
    expect(markTerminalSession).toHaveBeenCalledWith("tab-a")
    expect(screen.getByRole("region", { name: "region:Codex" })).toBeTruthy()
    expect(screen.queryByRole("button")).toBeNull()
    expect(screen.queryByRole("textbox")).toBeNull()
  })

  it("explains a missing CLI instead of spawning a terminal", () => {
    render(
      <SessionTerminalInput
        ownerKey="tab-a"
        agentType="codex"
        agentName="Codex"
        workingDir="/repo/codeg"
        isActive
        notice="Codex is not installed"
      />
    )

    expect(screen.getByRole("status").textContent).toContain(
      "Codex is not installed"
    )
    expect(screen.queryByTestId("session-terminal-view")).toBeNull()
    expect(terminalKill).not.toHaveBeenCalled()
    expect(markTerminalSession).not.toHaveBeenCalled()
  })

  it("keeps an unfocused split pane painted", async () => {
    render(
      <SessionTerminalInput
        ownerKey="tab-a"
        agentType="grok"
        agentName="Grok"
        workingDir="/repo/codeg"
        isActive={false}
        isVisible
      />
    )

    const view = await screen.findByTestId("session-terminal-view")
    expect(view.getAttribute("data-active")).toBe("false")
    expect(view.getAttribute("data-visible")).toBe("true")
    expect(view.getAttribute("data-keep")).toBe("true")
    expect(markTerminalSession).toHaveBeenCalledWith("tab-a")
  })

  it("settles the previous conversation when the agent changes", async () => {
    const view = render(
      <SessionTerminalInput
        ownerKey="tab-a"
        agentType="codex"
        agentName="Codex"
        workingDir="/repo/codeg"
        isActive
      />
    )
    const first = await screen.findByTestId("session-terminal-view")
    const firstId = first.getAttribute("data-id")
    expect(firstId).toBeTruthy()
    noteSessionTerminalConversation("tab-a", firstId ?? "", 9)

    view.rerender(
      <SessionTerminalInput
        ownerKey="tab-a"
        agentType="grok"
        agentName="Grok"
        workingDir="/repo/codeg"
        isActive
      />
    )

    await vi.waitFor(() => {
      expect(establishTerminalSession).toHaveBeenCalledWith(
        expect.objectContaining({
          observation: "process_exit",
          preferredConversationId: 9,
          terminalId: firstId,
          folderId: 0,
          externalId: "process-exit",
        })
      )
    })
    await vi.waitFor(() => {
      expect(terminalKill).toHaveBeenCalledWith(firstId)
    })
    expect(
      screen.getByTestId("session-terminal-view").getAttribute("data-id")
    ).not.toBe(firstId)
  })
})
