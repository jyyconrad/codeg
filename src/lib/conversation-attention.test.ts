import { afterEach, describe, expect, it } from "vitest"
import {
  conversationAttentionKind,
  conversationSeenEpoch,
  conversationShowsAttention,
  hydrateConversationAttention,
  markConversationAttentionSeen,
  resetConversationAttention,
  subscribeConversationAttention,
} from "./conversation-attention"

const UPDATED = "2026-09-15T00:00:00.000Z"

afterEach(() => {
  resetConversationAttention()
})

describe("conversationAttentionKind", () => {
  it("treats a finished turn as completed attention", () => {
    expect(conversationAttentionKind("pending_review")).toBe("completed")
    expect(conversationAttentionKind("completed")).toBe("completed")
  })

  it("treats cancelled as failed attention", () => {
    expect(conversationAttentionKind("cancelled")).toBe("failed")
  })

  it("treats in-progress (including a restart) as no attention", () => {
    expect(conversationAttentionKind("in_progress")).toBeNull()
    expect(conversationAttentionKind("pending")).toBeNull()
  })
})

describe("conversationShowsAttention", () => {
  it("shows completed attention until that status epoch has been opened", () => {
    expect(conversationShowsAttention("completed", 1, UPDATED, false)).toBe(
      "completed"
    )
    expect(
      conversationShowsAttention("pending_review", 1, UPDATED, false)
    ).toBe("completed")
  })

  it("hides completed attention while the row is selected", () => {
    expect(conversationShowsAttention("completed", 1, UPDATED, true)).toBeNull()
  })

  it("hides completed attention after that epoch is marked seen", () => {
    markConversationAttentionSeen(
      1,
      conversationSeenEpoch("completed", UPDATED)
    )
    expect(
      conversationShowsAttention("completed", 1, UPDATED, false)
    ).toBeNull()
  })

  it("shows completed attention again when the conversation finishes a new turn", () => {
    markConversationAttentionSeen(
      1,
      conversationSeenEpoch("completed", UPDATED)
    )
    expect(
      conversationShowsAttention(
        "pending_review",
        1,
        "2026-09-15T01:00:00.000Z",
        false
      )
    ).toBe("completed")
  })

  it("shows failed attention until opened, then hides the unread mark", () => {
    expect(conversationShowsAttention("cancelled", 2, UPDATED, false)).toBe(
      "failed"
    )
    markConversationAttentionSeen(
      2,
      conversationSeenEpoch("cancelled", UPDATED)
    )
    expect(
      conversationShowsAttention("cancelled", 2, UPDATED, false)
    ).toBeNull()
  })

  it("drops failed attention when the conversation is restarted", () => {
    expect(conversationShowsAttention("cancelled", 3, UPDATED, false)).toBe(
      "failed"
    )
    expect(
      conversationShowsAttention(
        "in_progress",
        3,
        "2026-09-15T02:00:00.000Z",
        false
      )
    ).toBeNull()
  })

  it("can persist a seen epoch without notifying subscribers", () => {
    let notified = 0
    const stop = subscribeConversationAttention(() => {
      notified += 1
    })
    markConversationAttentionSeen(
      5,
      conversationSeenEpoch("completed", UPDATED),
      { notify: false }
    )
    expect(notified).toBe(0)
    expect(
      conversationShowsAttention("completed", 5, UPDATED, false)
    ).toBeNull()
    stop()
  })

  it("restores seen epochs from localStorage on hydrate", () => {
    markConversationAttentionSeen(
      4,
      conversationSeenEpoch("completed", UPDATED)
    )
    const raw = window.localStorage.getItem(
      "codeg.conversation-attention.seen.v1"
    )
    resetConversationAttention()
    expect(conversationShowsAttention("completed", 4, UPDATED, false)).toBe(
      "completed"
    )
    if (raw)
      window.localStorage.setItem("codeg.conversation-attention.seen.v1", raw)
    hydrateConversationAttention()
    expect(
      conversationShowsAttention("completed", 4, UPDATED, false)
    ).toBeNull()
  })
})
