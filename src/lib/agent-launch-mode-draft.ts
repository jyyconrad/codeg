import type { AgentLaunchMode } from "@/lib/types"

export function rebaseAgentLaunchMode<
  T extends { launchMode: AgentLaunchMode },
>(draft: T, backendMode: AgentLaunchMode | undefined, savePending: boolean): T {
  if (savePending) return draft
  return { ...draft, launchMode: backendMode ?? "acp" }
}
