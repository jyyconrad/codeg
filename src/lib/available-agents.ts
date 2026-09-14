import type { AcpAgentInfo, AgentType } from "@/lib/types"

/** Fields the "usable in pickers / assignment UIs" gate actually reads. */
export type AvailableAgentGate = Pick<
  AcpAgentInfo,
  "enabled" | "available" | "installed_version"
>

/**
 * Agents that belong on pickers and assignment UIs (home-page composer, MCP
 * target apps, Skills, skill packs).
 *
 * `AcpAgentInfo.available` is only the platform gate. Combined with the
 * settings enable toggle and a detected install, this is the product sense of
 * "可用": the Agents settings page still lists every agent so the rest can be
 * installed or turned on.
 */
export function isAvailableAgent(agent: AvailableAgentGate): boolean {
  return agent.enabled && agent.available && agent.installed_version != null
}

export function filterAvailableAgents<T extends AvailableAgentGate>(
  agents: readonly T[]
): T[] {
  return agents.filter(isAvailableAgent)
}

export function availableAgentTypeSet(
  agents: readonly (AvailableAgentGate & { agent_type: AgentType })[]
): Set<AgentType> {
  return new Set(filterAvailableAgents(agents).map((agent) => agent.agent_type))
}

/**
 * Keep catalog entries whose `value` is an available agent type. Used by the
 * MCP "enable for apps" checkbox grids, which are a closed list of built-in
 * app ids rather than the live `AcpAgentInfo[]`.
 */
export function filterOptionsByAvailableAgents<T extends { value: string }>(
  options: readonly T[],
  agents: readonly (AvailableAgentGate & { agent_type: AgentType })[]
): T[] {
  const types = availableAgentTypeSet(agents)
  return options.filter((option) => types.has(option.value as AgentType))
}
