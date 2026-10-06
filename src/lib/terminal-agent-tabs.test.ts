import { describe, expect, it } from "vitest"
import {
  canRestoreAgentTerminalTab,
  parseStoredAgentTerminalTabs,
  serializeAgentTerminalTabs,
  waitForRestorableAgentTerminal,
  markStoredAgentTerminalLaunched,
} from "@/lib/terminal-agent-tabs"

describe("Agent terminal tab session storage", () => {
  it("persists attach metadata without prompt or run credentials", () => {
    const serialized = serializeAgentTerminalTabs([
      {
        id: "term-1",
        title: "Codex · Terminal",
        workingDir: "/repo",
        shell: "/bin/zsh",
        agentType: "codex",
      },
    ])

    expect(serialized).toContain("term-1")
    expect(serialized).not.toContain("prompt")
    expect(serialized).not.toContain("run")
    expect(serialized).not.toContain("token")
  })

  it("restores only an active run owned by the stored agent", () => {
    const tab = {
      id: "term-1",
      title: "Codex",
      workingDir: "/repo",
      agentType: "codex" as const,
    }
    const state = {
      terminal_id: "term-1",
      run_id: "run-1",
      hook_support: "available" as const,
      latest_event: null,
      active: true,
      agent_type: "codex" as const,
    }

    expect(canRestoreAgentTerminalTab(tab, { alive: true }, state)).toBe(true)
    expect(
      canRestoreAgentTerminalTab(
        tab,
        { alive: true },
        {
          ...state,
          active: false,
        }
      )
    ).toBe(false)
    expect(
      canRestoreAgentTerminalTab(
        tab,
        { alive: true },
        {
          ...state,
          agent_type: "claude_code",
        }
      )
    ).toBe(false)
  })

  it("waits for a pending launch to create its PTY without spawning again", async () => {
    const tab = {
      id: "term-1",
      title: "Codex",
      workingDir: "/repo",
      agentType: "codex" as const,
    }
    let attempt = 0
    const probe = async () => {
      attempt += 1
      return {
        snapshot: { alive: attempt >= 3, data: "", seq: 0 },
        state: {
          terminal_id: "term-1",
          run_id: "run-1",
          hook_support: "available" as const,
          latest_event: null,
          active: true,
          agent_type: "codex" as const,
        },
      }
    }

    await expect(
      waitForRestorableAgentTerminal(tab, probe, {
        attempts: 3,
        intervalMs: 0,
      })
    ).resolves.toMatchObject({ snapshot: { alive: true } })
    expect(attempt).toBe(3)
  })

  it("converts a launched tab to attach-only and drops its prompt", () => {
    const tab = {
      id: "term-1",
      folderId: 1,
      title: "Codex",
      workingDir: "/repo",
      agentType: "codex" as const,
      agentLaunch: { agentType: "codex" as const, prompt: "secret task" },
    }
    const launched = markStoredAgentTerminalLaunched(tab, {
      runId: "run-1",
      hookSupport: "available",
    })

    expect(launched.agentAttach).toBe(true)
    expect("agentLaunch" in launched).toBe(false)
  })

  it("waits beyond the five-second probe boundary by default", async () => {
    const tab = {
      id: "term-slow",
      title: "Codex",
      workingDir: "/repo",
      agentType: "codex" as const,
    }
    let attempt = 0
    const restored = await waitForRestorableAgentTerminal(
      tab,
      async () => {
        attempt += 1
        return {
          snapshot: { alive: attempt >= 60, data: "", seq: 0 },
          state: {
            terminal_id: "term-slow",
            run_id: "run-slow",
            hook_support: "available" as const,
            latest_event: null,
            active: true,
            agent_type: "codex" as const,
          },
        }
      },
      { intervalMs: 0 }
    )

    expect(restored?.snapshot.alive).toBe(true)
    expect(attempt).toBe(60)
  })

  it("drops malformed records instead of restoring an unsafe tab", () => {
    expect(
      parseStoredAgentTerminalTabs(
        JSON.stringify([
          {
            id: "term-1",
            title: "Codex",
            workingDir: "/repo",
            agentType: "codex",
          },
          { id: "missing-dir", title: "Bad", agentType: "codex" },
          {
            id: "wrong-agent",
            title: "Bad",
            workingDir: "/repo",
            agentType: 42,
          },
        ])
      )
    ).toEqual([
      {
        id: "term-1",
        title: "Codex",
        workingDir: "/repo",
        agentType: "codex",
      },
    ])
  })
})
