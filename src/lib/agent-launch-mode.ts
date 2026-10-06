import type { AgentLaunchMode } from "@/lib/types"

export function newSessionLaunchRoute(
  launchMode: AgentLaunchMode | null
): AgentLaunchMode | "wait" {
  return launchMode ?? "wait"
}
