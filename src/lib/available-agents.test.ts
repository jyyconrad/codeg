import { describe, expect, it } from "vitest"

import {
  availableAgentTypeSet,
  filterAvailableAgents,
  filterOptionsByAvailableAgents,
  isAvailableAgent,
} from "./available-agents"
import type { AgentType } from "@/lib/types"

function agent(
  overrides: {
    enabled?: boolean
    available?: boolean
    installed_version?: string | null
    agent_type?: AgentType
  } = {}
) {
  return {
    agent_type: overrides.agent_type ?? ("codex" as AgentType),
    enabled: overrides.enabled ?? true,
    available: overrides.available ?? true,
    installed_version:
      overrides.installed_version !== undefined
        ? overrides.installed_version
        : "1.0.0",
  }
}

describe("isAvailableAgent", () => {
  it("accepts an enabled, platform-available, installed agent", () => {
    expect(isAvailableAgent(agent())).toBe(true)
  })

  it("rejects a disabled agent even when it is installed", () => {
    expect(isAvailableAgent(agent({ enabled: false }))).toBe(false)
  })

  it("rejects a platform-unavailable agent", () => {
    expect(isAvailableAgent(agent({ available: false }))).toBe(false)
  })

  it("rejects an uninstalled agent", () => {
    expect(isAvailableAgent(agent({ installed_version: null }))).toBe(false)
  })
})

describe("filterAvailableAgents", () => {
  it("keeps only agents that pass the gate, in input order", () => {
    const agents = [
      agent({ agent_type: "claude_code", enabled: false }),
      agent({ agent_type: "codex" }),
      agent({ agent_type: "gemini", available: false }),
      agent({ agent_type: "open_code", installed_version: null }),
      agent({ agent_type: "grok" }),
    ]
    expect(filterAvailableAgents(agents).map((a) => a.agent_type)).toEqual([
      "codex",
      "grok",
    ])
  })
})

describe("filterOptionsByAvailableAgents", () => {
  it("drops catalog options whose agent is not available", () => {
    const options = [
      { value: "claude_code", label: "Claude Code" },
      { value: "codex", label: "Codex CLI" },
      { value: "open_code", label: "OpenCode" },
    ]
    const agents = [
      agent({ agent_type: "codex" }),
      agent({ agent_type: "open_code", enabled: false }),
      agent({ agent_type: "claude_code", installed_version: null }),
    ]
    expect(
      filterOptionsByAvailableAgents(options, agents).map((o) => o.value)
    ).toEqual(["codex"])
  })

  it("returns an empty list when no catalog agent is available", () => {
    const options = [{ value: "codex", label: "Codex CLI" }]
    expect(filterOptionsByAvailableAgents(options, [])).toEqual([])
  })
})

describe("availableAgentTypeSet", () => {
  it("is a set of the surviving agent types", () => {
    const types = availableAgentTypeSet([
      agent({ agent_type: "codex" }),
      agent({ agent_type: "gemini", available: false }),
    ])
    expect(types.has("codex")).toBe(true)
    expect(types.has("gemini")).toBe(false)
  })
})
