import type { AgentTerminalEvent } from "@/lib/types"

export function newerAgentTerminalEvent(
  left: AgentTerminalEvent | null | undefined,
  right: AgentTerminalEvent | null | undefined
): AgentTerminalEvent | undefined {
  if (!left) return right ?? undefined
  if (!right) return left
  return Date.parse(right.received_at) >= Date.parse(left.received_at)
    ? right
    : left
}

export function shouldKillTerminalAfterCancelledMount(
  attach: boolean,
  spawnedByThisMount: boolean,
  agentTerminal: boolean,
  explicitlyClosed: boolean
): boolean {
  if (!agentTerminal) return !attach
  return spawnedByThisMount && explicitlyClosed
}
