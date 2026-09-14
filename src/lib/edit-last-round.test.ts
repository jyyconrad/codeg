import { describe, expect, it } from "vitest"

import {
  agentSupportsNamedFork,
  lastRoundEditTarget,
  namedAssistantForkTurnId,
  userTurnToPromptDraft,
} from "./edit-last-round"
import type { AgentType, MessageTurn } from "./types"

function userTurn(
  id: string,
  text: string,
  extra?: Partial<MessageTurn>
): MessageTurn {
  return {
    id,
    role: "user",
    blocks: [{ type: "text", text }],
    timestamp: "2026-09-11T00:00:00.000Z",
    ...extra,
  }
}

function assistantTurn(
  id: string,
  text: string,
  extra?: Partial<MessageTurn>
): MessageTurn {
  return {
    id,
    role: "assistant",
    blocks: text ? [{ type: "text", text }] : [],
    timestamp: "2026-09-11T00:00:01.000Z",
    ...extra,
  }
}

describe("agentSupportsNamedFork", () => {
  it("is true only for Claude, Codex and DeepSeek", () => {
    expect(agentSupportsNamedFork("claude_code")).toBe(true)
    expect(agentSupportsNamedFork("codex")).toBe(true)
    expect(agentSupportsNamedFork("deepseek")).toBe(true)
    for (const agent of [
      "gemini",
      "grok",
      "qoder",
      "kimi_code",
      "open_code",
      "cursor",
      "codeg_agent",
      "custom:acme",
    ] as AgentType[]) {
      expect(agentSupportsNamedFork(agent)).toBe(false)
    }
  })
})

describe("namedAssistantForkTurnId", () => {
  it("rejects live ids that the parser has not named yet", () => {
    const turn = assistantTurn("live-1-abc", "hello")
    expect(namedAssistantForkTurnId(turn, "claude_code")).toBeNull()
  })

  it("uses the backfilled parser id when the live id is still on the turn", () => {
    const turn = assistantTurn("live-1-abc", "hello", {
      source_turn_id: "turn-2",
    })
    expect(namedAssistantForkTurnId(turn, "claude_code")).toBe("turn-2")
  })

  it("rejects a Codex assistant bubble with no text to fingerprint", () => {
    const turn = assistantTurn("turn-2", "")
    expect(namedAssistantForkTurnId(turn, "codex")).toBeNull()
  })
})

describe("lastRoundEditTarget", () => {
  const history = [
    { turn: userTurn("turn-0", "first") },
    { turn: assistantTurn("turn-1", "ok", { agent_message_id: "msg_01" }) },
    { turn: userTurn("turn-2", "second") },
    { turn: assistantTurn("turn-3", "done", { agent_message_id: "msg_02" }) },
  ]

  it("forks at the assistant turn before the last user message", () => {
    const target = lastRoundEditTarget(history, "claude_code")
    expect(target?.userTurn.id).toBe("turn-2")
    expect(target?.forkFromTurnId).toBe("turn-1")
    expect(target?.draft.displayText).toBe("second")
  })

  it("works after an error with no assistant reply on the last round", () => {
    const target = lastRoundEditTarget(history.slice(0, 3), "codex")
    expect(target?.userTurn.id).toBe("turn-2")
    expect(target?.forkFromTurnId).toBe("turn-1")
  })

  it("rewinds the first user message to an empty session, even after the assistant has replied", () => {
    const target = lastRoundEditTarget(
      [
        { turn: userTurn("turn-0", "only") },
        { turn: assistantTurn("turn-1", "ok") },
      ],
      "claude_code"
    )
    expect(target?.userTurn.id).toBe("turn-0")
    expect(target?.forkFromTurnId).toBeNull()
    expect(target?.draft.displayText).toBe("only")
  })

  it("rewinds the first user message on agents that cannot named-fork", () => {
    const firstRound = [
      { turn: userTurn("turn-0", "only") },
      { turn: assistantTurn("turn-1", "ok") },
    ]
    for (const agent of ["codeg_agent", "grok", "qoder"] as AgentType[]) {
      const target = lastRoundEditTarget(firstRound, agent)
      expect(target?.userTurn.id).toBe("turn-0")
      expect(target?.forkFromTurnId).toBeNull()
    }
  })

  it("hides later rounds for agents that only tail-fork", () => {
    expect(lastRoundEditTarget(history, "qoder")).toBeNull()
    expect(lastRoundEditTarget(history, "grok")).toBeNull()
    expect(lastRoundEditTarget(history, "codeg_agent")).toBeNull()
  })

  it("hides while the last user turn is still optimistic", () => {
    expect(
      lastRoundEditTarget(
        [
          ...history.slice(0, 2),
          { turn: userTurn("opt-1", "pending"), phase: "optimistic" },
        ],
        "claude_code"
      )
    ).toBeNull()
  })

  it("hides while the reply after the last user turn is still in flight", () => {
    expect(
      lastRoundEditTarget(
        [
          ...history.slice(0, 3),
          {
            turn: assistantTurn("turn-3", "…"),
            phase: "streaming",
            isInFlightRound: true,
          },
        ],
        "claude_code"
      )
    ).toBeNull()
  })
})

describe("userTurnToPromptDraft", () => {
  it("keeps text and images, drops tool blocks", () => {
    const draft = userTurnToPromptDraft({
      id: "turn-2",
      role: "user",
      timestamp: "",
      blocks: [
        {
          type: "image",
          data: "aaa",
          mime_type: "image/png",
          uri: "file:///x.png",
        },
        { type: "text", text: "look" },
        {
          type: "tool_use",
          tool_use_id: "t",
          tool_name: "x",
          input_preview: null,
        },
      ],
    })
    expect(draft).toEqual({
      displayText: "look",
      blocks: [
        {
          type: "image",
          data: "aaa",
          mime_type: "image/png",
          uri: "file:///x.png",
        },
        { type: "text", text: "look" },
      ],
    })
  })

  it("returns null when there is nothing to resend", () => {
    expect(
      userTurnToPromptDraft({
        id: "turn-2",
        role: "user",
        timestamp: "",
        blocks: [],
      })
    ).toBeNull()
  })
})
