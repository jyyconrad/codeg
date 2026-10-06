import type {
  AgentType,
  AgentTerminalEvent,
  TerminalAgentHookSupport,
  TerminalAgentState,
  TerminalSnapshot,
} from "@/lib/types"

export interface StoredAgentTerminalTab {
  id: string
  title: string
  workingDir: string
  shell?: string
  agentType: AgentType
}

export function markStoredAgentTerminalLaunched<
  T extends { agentLaunch?: unknown },
>(
  tab: T,
  run: {
    runId: string
    hookSupport: TerminalAgentHookSupport
    latestEvent?: AgentTerminalEvent
  }
): Omit<T, "agentLaunch"> & {
  agentAttach: true
  agentRun: {
    runId: string
    hookSupport: TerminalAgentHookSupport
    latestEvent?: AgentTerminalEvent
    latestEventType?: AgentTerminalEvent["type"]
  }
} {
  const rest = Object.fromEntries(
    Object.entries(tab).filter(([key]) => key !== "agentLaunch")
  ) as Omit<T, "agentLaunch">
  return {
    ...rest,
    agentAttach: true,
    agentRun: {
      ...run,
      latestEventType: run.latestEvent?.type,
    },
  }
}

export function serializeAgentTerminalTabs(
  tabs: readonly StoredAgentTerminalTab[]
): string {
  return JSON.stringify(tabs)
}

export function parseStoredAgentTerminalTabs(
  raw: string | null
): StoredAgentTerminalTab[] {
  if (!raw) return []
  try {
    const value: unknown = JSON.parse(raw)
    if (!Array.isArray(value)) return []
    return value.flatMap((item) => {
      if (!item || typeof item !== "object") return []
      const record = item as Record<string, unknown>
      if (
        typeof record.id !== "string" ||
        typeof record.title !== "string" ||
        typeof record.workingDir !== "string" ||
        typeof record.agentType !== "string" ||
        (record.shell != null && typeof record.shell !== "string")
      ) {
        return []
      }
      return [
        {
          id: record.id,
          title: record.title,
          workingDir: record.workingDir,
          ...(record.shell ? { shell: record.shell } : {}),
          agentType: record.agentType as AgentType,
        },
      ]
    })
  } catch {
    return []
  }
}

export function canRestoreAgentTerminalTab(
  tab: StoredAgentTerminalTab,
  snapshot: Pick<TerminalSnapshot, "alive"> | null,
  state: TerminalAgentState | null
): boolean {
  return Boolean(
    snapshot?.alive &&
    state?.active === true &&
    state.agent_type === tab.agentType
  )
}

export async function waitForRestorableAgentTerminal(
  tab: StoredAgentTerminalTab,
  probe: () => Promise<{
    snapshot: TerminalSnapshot | null
    state: TerminalAgentState | null
  }>,
  options: { attempts?: number; intervalMs?: number } = {}
): Promise<{
  snapshot: TerminalSnapshot
  state: TerminalAgentState
} | null> {
  const attempts = options.attempts ?? 100
  const intervalMs = options.intervalMs ?? 100
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    const result = await probe()
    if (canRestoreAgentTerminalTab(tab, result.snapshot, result.state)) {
      return {
        snapshot: result.snapshot as TerminalSnapshot,
        state: result.state as TerminalAgentState,
      }
    }
    // A mismatched active run means this terminal id has been reused. Waiting
    // cannot make it belong to the stored Agent again.
    if (
      result.state?.active === true &&
      result.state.agent_type !== tab.agentType
    ) {
      return null
    }
    if (result.state?.active === false) return null
    if (attempt + 1 < attempts && intervalMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, intervalMs))
    }
  }
  return null
}
