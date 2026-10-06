import { describe, expect, it } from "vitest"
import { rebaseAgentLaunchMode } from "@/lib/agent-launch-mode-draft"

describe("rebaseAgentLaunchMode", () => {
  it("rebases the backend mode while preserving other unsaved fields", () => {
    expect(
      rebaseAgentLaunchMode(
        { launchMode: "acp" as const, envText: "UNSAVED=1" },
        "terminal",
        false
      )
    ).toEqual({ launchMode: "terminal", envText: "UNSAVED=1" })
  })

  it("keeps the optimistic local mode while a save is pending", () => {
    const draft = { launchMode: "terminal" as const, envText: "UNSAVED=1" }
    expect(rebaseAgentLaunchMode(draft, "acp", true)).toBe(draft)
  })
})
