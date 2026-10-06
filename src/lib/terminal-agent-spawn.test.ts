import { describe, expect, it, vi } from "vitest"
import { startTerminalAgentOnce } from "@/lib/terminal-agent-spawn"

describe("startTerminalAgentOnce", () => {
  it("shares one live spawn request across remounted views", async () => {
    let resolve!: (value: string) => void
    const start = vi.fn(() => new Promise<string>((done) => (resolve = done)))

    const first = startTerminalAgentOnce("term-1", start)
    const remount = startTerminalAgentOnce("term-1", start)
    expect(first.owner).toBe(true)
    expect(remount.owner).toBe(false)
    expect(start).toHaveBeenCalledTimes(1)

    resolve("run-1")
    await expect(first.promise).resolves.toBe("run-1")
    await expect(remount.promise).resolves.toBe("run-1")
  })
})
