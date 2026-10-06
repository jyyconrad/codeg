import { beforeEach, describe, expect, it, vi } from "vitest"

const call = vi.hoisted(() => vi.fn())

vi.mock("@/lib/transport", () => ({
  getTransport: () => ({ call }),
}))

import { terminalSpawnAgent } from "@/lib/api"

describe("terminalSpawnAgent", () => {
  beforeEach(() => call.mockReset())

  it("sends the initial task as structured data instead of a shell command", async () => {
    call.mockResolvedValue({
      terminal_id: "term-1",
      run_id: "run-1",
      hook_support: "available",
    })

    await terminalSpawnAgent({
      agentType: "codex",
      workingDir: "/repo with spaces",
      prompt: '修复 "登录"\n然后运行 $(danger)',
      terminalId: "term-1",
      shell: "/bin/zsh",
    })

    expect(call).toHaveBeenCalledWith("terminal_spawn_agent", {
      agentType: "codex",
      workingDir: "/repo with spaces",
      prompt: '修复 "登录"\n然后运行 $(danger)',
      terminalId: "term-1",
      shell: "/bin/zsh",
    })
  })
})
