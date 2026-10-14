import { afterEach, describe, expect, it } from "vitest"
import {
  claimSessionTerminal,
  noteSessionTerminalConversation,
  releaseSessionTerminal,
  sessionTerminalBinding,
  sessionTerminalOwnerByTerminal,
  terminalLaunchDirectory,
} from "@/lib/session-terminal-session"

const owners: string[] = []

afterEach(() => {
  for (const owner of owners) releaseSessionTerminal(owner)
  owners.length = 0
})

function claim(owner: string, agent: "claude-code" | "codex", dir: string) {
  owners.push(owner)
  return claimSessionTerminal(owner, agent, dir)
}

describe("terminalLaunchDirectory", () => {
  it("uses the live folder path when a draft still stores another directory", () => {
    expect(terminalLaunchDirectory("/repo/codeg", "/repo/old")).toBe(
      "/repo/codeg"
    )
  })

  it("keeps a scratch directory when the tab has no folder path", () => {
    expect(terminalLaunchDirectory("  ", "/tmp/chat-scratch")).toBe(
      "/tmp/chat-scratch"
    )
    expect(terminalLaunchDirectory(null, "  ")).toBeUndefined()
  })
})

describe("session terminal ownership", () => {
  it("keeps the same terminal while the agent and directory stay", () => {
    const first = claim("tab-1", "claude-code", "/repo")
    const second = claimSessionTerminal("tab-1", "claude-code", "/repo")

    expect(second.terminalId).toBe(first.terminalId)
    expect(second.replacedId).toBeNull()
  })

  it("replaces the terminal when the agent changes", () => {
    const first = claim("tab-2", "claude-code", "/repo")
    const second = claimSessionTerminal("tab-2", "codex", "/repo")

    expect(second.terminalId).not.toBe(first.terminalId)
    expect(second.replacedId).toBe(first.terminalId)
  })

  it("replaces the terminal when the directory changes", () => {
    const first = claim("tab-3", "codex", "/repo")
    const second = claimSessionTerminal("tab-3", "codex", "/other")

    expect(second.replacedId).toBe(first.terminalId)
  })

  it("finds the owner of a live terminal and forgets a replaced one", () => {
    const first = claim("tab-6", "codex", "/repo")
    expect(sessionTerminalOwnerByTerminal(first.terminalId)).toBe("tab-6")

    const second = claimSessionTerminal("tab-6", "claude_code", "/repo")
    expect(sessionTerminalOwnerByTerminal(first.terminalId)).toBeNull()
    expect(sessionTerminalOwnerByTerminal(second.terminalId)).toBe("tab-6")

    releaseSessionTerminal("tab-6")
    expect(sessionTerminalOwnerByTerminal(second.terminalId)).toBeNull()
  })

  it("keeps a bound conversation until the terminal is replaced", () => {
    const first = claim("tab-7", "codex", "/repo")
    noteSessionTerminalConversation("tab-7", first.terminalId, 12)
    const again = claimSessionTerminal("tab-7", "codex", "/repo")

    expect(again.replacedId).toBeNull()
    expect(again.replacedConversationId).toBeNull()
    expect(sessionTerminalBinding("tab-7")?.conversationId).toBe(12)

    const next = claimSessionTerminal("tab-7", "claude-code", "/repo")
    expect(next.replacedId).toBe(first.terminalId)
    expect(next.replacedConversationId).toBe(12)
    expect(sessionTerminalBinding("tab-7")?.conversationId).toBeNull()
    noteSessionTerminalConversation("tab-7", first.terminalId, 12)
    expect(sessionTerminalBinding("tab-7")?.conversationId).toBeNull()
  })

  it("does not share a terminal across sessions", () => {
    const left = claim("tab-4", "codex", "/repo")
    const right = claim("tab-5", "codex", "/repo")

    expect(left.terminalId).not.toBe(right.terminalId)
  })
})
