import { describe, expect, it } from "vitest"
import { terminalAgentPrompt } from "@/lib/terminal-agent-draft"

describe("terminalAgentPrompt", () => {
  it("returns complete plain text for a native CLI argument", () => {
    expect(
      terminalAgentPrompt(
        {
          blocks: [{ type: "text", text: "修复登录\n然后测试" }],
          displayText: "修复登录\n然后测试",
        },
        "attachments"
      )
    ).toBe("修复登录\n然后测试")
  })

  it("rejects a draft that requires structured blocks", () => {
    expect(
      terminalAgentPrompt(
        {
          blocks: [
            { type: "text", text: "inspect" },
            {
              type: "resource_link",
              uri: "file:///repo/report.pdf",
              name: "report.pdf",
            },
          ],
          displayText: "inspect [report.pdf](file:///repo/report.pdf)",
        },
        "attachments"
      )
    ).toBeNull()
  })
})
