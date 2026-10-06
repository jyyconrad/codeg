import { describe, expect, it } from "vitest"
import type { AgentTerminalEvent } from "@/lib/types"
import { claimFromTerminalEvent } from "@/lib/terminal-session-claim"

function event(
  overrides: Partial<AgentTerminalEvent> = {}
): AgentTerminalEvent {
  return {
    terminal_id: "term-1",
    agent_type: "claude_code",
    run_id: "run-1",
    event_id: "event-1",
    type: "session_started",
    native_event: "SessionStart",
    native_session_id: "sid-1",
    received_at: "2026-09-30T00:00:00Z",
    ...overrides,
  }
}

describe("claimFromTerminalEvent", () => {
  it("reads a session id and title from SessionStart", () => {
    expect(
      claimFromTerminalEvent(
        event({ title: " Plan ", model: "opus", title_seed: "  " })
      )
    ).toEqual({
      externalId: "sid-1",
      title: "Plan",
      titleSeed: undefined,
      model: "opus",
    })
  })

  it("accepts a Codex turn that is the first event carrying a session id", () => {
    expect(
      claimFromTerminalEvent(
        event({
          type: "turn_started",
          native_event: "UserPromptSubmit",
          title: undefined,
          title_seed: "fix the pane",
        })
      )
    ).toEqual({
      externalId: "sid-1",
      title: undefined,
      titleSeed: "fix the pane",
      model: undefined,
    })
  })

  it("accepts a session end that still names the session", () => {
    expect(
      claimFromTerminalEvent(
        event({
          type: "session_ended",
          native_event: "SessionEnd",
        })
      )
    ).toEqual({
      externalId: "sid-1",
      title: undefined,
      titleSeed: undefined,
      model: undefined,
    })
  })

  it("ignores an event with no session id", () => {
    expect(
      claimFromTerminalEvent(event({ native_session_id: "  " }))
    ).toBeNull()
    expect(
      claimFromTerminalEvent(event({ native_session_id: undefined }))
    ).toBeNull()
  })
})
