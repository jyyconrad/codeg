import { describe, expect, it } from "vitest"

import type { AgentType, ModelProviderInfo } from "@/lib/types"

import {
  CODEG_PROVIDER_PRESETS,
  codegProviderPreset,
  isLoopbackHttpUrl,
  modelProviderOptionLabel,
  modelProvidersForAgent,
} from "./codeg-agent-providers"

function provider(
  overrides: Partial<ModelProviderInfo> &
    Pick<ModelProviderInfo, "id" | "agent_type">
): ModelProviderInfo {
  return {
    name: `p-${overrides.id}`,
    api_url: "https://example.test/v1",
    api_key: "sk",
    api_key_masked: "sk",
    model: "m",
    created_at: "",
    updated_at: "",
    ...overrides,
  }
}

describe("Codeg Completions provider presets", () => {
  it("covers cloud, local loopback, and custom /v1 endpoints", () => {
    const byId = Object.fromEntries(
      CODEG_PROVIDER_PRESETS.map((preset) => [preset.id, preset])
    )
    expect(byId.openai?.apiUrl).toBe("https://api.openai.com/v1")
    expect(byId.deepseek?.apiUrl).toBe("https://api.deepseek.com/v1")
    expect(byId.groq?.apiUrl).toBe("https://api.groq.com/openai/v1")
    expect(byId.together?.apiUrl).toBe("https://api.together.xyz/v1")
    expect(byId.openrouter?.apiUrl).toBe("https://openrouter.ai/api/v1")
    expect(byId.ollama).toMatchObject({
      apiUrl: "http://127.0.0.1:11434/v1",
      local: true,
    })
    expect(byId.lmstudio).toMatchObject({
      apiUrl: "http://127.0.0.1:1234/v1",
      local: true,
    })
    expect(byId.custom?.apiUrl).toBe("")
    expect(codegProviderPreset("missing")).toBeUndefined()
  })

  it("treats loopback hosts as local (API key optional)", () => {
    expect(isLoopbackHttpUrl("http://127.0.0.1:11434/v1")).toBe(true)
    expect(isLoopbackHttpUrl("http://localhost:1234/v1")).toBe(true)
    expect(isLoopbackHttpUrl("http://[::1]:11434/v1")).toBe(true)
    expect(isLoopbackHttpUrl("https://0.0.0.0:1234/v1")).toBe(true)
    expect(isLoopbackHttpUrl("https://api.openai.com/v1")).toBe(false)
    expect(isLoopbackHttpUrl("not-a-url")).toBe(false)
  })

  it("lets Codeg Agent bind Claude / Codex / Gemini / Codeg channels", () => {
    const providers = [
      provider({ id: 1, agent_type: "claude_code" }),
      provider({ id: 2, agent_type: "gemini" }),
      provider({ id: 3, agent_type: "codeg_agent", name: "Ollama" }),
      provider({ id: 4, agent_type: "grok" }),
    ]
    expect(
      modelProvidersForAgent("codeg_agent" as AgentType, providers).map(
        (row) => row.id
      )
    ).toEqual([1, 2, 3])
    expect(
      modelProviderOptionLabel(providers[0], "codeg_agent" as AgentType)
    ).toContain("Claude")
    expect(
      modelProviderOptionLabel(providers[2], "codeg_agent" as AgentType)
    ).toBe("Ollama")
  })
})
