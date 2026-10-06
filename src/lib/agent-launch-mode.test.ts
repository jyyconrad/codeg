import { describe, expect, it } from "vitest"
import { newSessionLaunchRoute } from "@/lib/agent-launch-mode"

describe("newSessionLaunchRoute", () => {
  it("waits instead of defaulting to ACP while configuration is loading", () => {
    expect(newSessionLaunchRoute(null)).toBe("wait")
  })

  it("preserves the configured launch route", () => {
    expect(newSessionLaunchRoute("acp")).toBe("acp")
    expect(newSessionLaunchRoute("terminal")).toBe("terminal")
  })
})
