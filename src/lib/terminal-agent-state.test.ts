import { describe, expect, it } from "vitest"
import {
  newerAgentTerminalEvent,
  shouldKillTerminalAfterCancelledMount,
} from "@/lib/terminal-agent-state"
import type { AgentTerminalEvent } from "@/lib/types"

function event(
  eventId: string,
  receivedAt: string,
  type: AgentTerminalEvent["type"]
): AgentTerminalEvent {
  return {
    terminal_id: "term-1",
    agent_type: "codex",
    run_id: "run-1",
    event_id: eventId,
    type,
    native_event: type,
    received_at: receivedAt,
  }
}

describe("newerAgentTerminalEvent", () => {
  it("keeps a live event that arrived after the queried snapshot", () => {
    const snapshot = event(
      "snapshot",
      "2026-09-30T10:00:00.000Z",
      "turn_started"
    )
    const live = event("live", "2026-09-30T10:00:01.000Z", "turn_stopped")

    expect(newerAgentTerminalEvent(snapshot, live)).toBe(live)
  })

  it("uses the snapshot when it is newer than the buffered event", () => {
    const live = event("live", "2026-09-30T10:00:00.000Z", "turn_started")
    const snapshot = event(
      "snapshot",
      "2026-09-30T10:00:01.000Z",
      "turn_stopped"
    )

    expect(newerAgentTerminalEvent(live, snapshot)).toBe(snapshot)
  })
})

describe("shouldKillTerminalAfterCancelledMount", () => {
  it("kills an explicitly closed Agent spawn but preserves it on transient unmount", () => {
    expect(shouldKillTerminalAfterCancelledMount(true, true, true, true)).toBe(
      true
    )
    expect(shouldKillTerminalAfterCancelledMount(true, true, true, false)).toBe(
      false
    )
    expect(shouldKillTerminalAfterCancelledMount(true, false, true, true)).toBe(
      false
    )
    expect(
      shouldKillTerminalAfterCancelledMount(false, false, false, false)
    ).toBe(true)
  })
})
