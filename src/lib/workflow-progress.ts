/**
 * Canonical workflow-run merge — the client half of `SessionState::apply_event`
 * for `AcpEvent::Workflow`. Same contract as `lib/async-tasks.ts`: only a
 * `spawned` delta may CREATE a row; absent fields mean unchanged; terminal
 * rows are retained so a later correction can revise them.
 */

import type {
  WorkflowAgent,
  WorkflowDelta,
  WorkflowPhase,
  WorkflowRun,
} from "@/lib/types"

const TERMINAL_STATES = new Set(["completed", "failed", "stopped"])

export function isWorkflowTerminal(run: WorkflowRun): boolean {
  return TERMINAL_STATES.has(run.state)
}

function recordFromDelta(delta: WorkflowDelta): WorkflowRun {
  return {
    run_id: delta.run_id,
    name: delta.name ?? "Workflow",
    objective: delta.objective ?? null,
    state: delta.state ?? "running",
    phases: delta.phases ?? [],
    current_phase: delta.current_phase ?? null,
    agents: delta.agents ?? [],
    agents_done: delta.agents_done ?? 0,
    agents_running: delta.agents_running ?? 0,
    agents_used: delta.agents_used ?? 0,
    agent_budget: delta.agent_budget ?? null,
    agents_remaining: delta.agents_remaining ?? null,
    elapsed_ms: delta.elapsed_ms ?? null,
    last_event: delta.last_event ?? null,
    last_event_detail: delta.last_event_detail ?? null,
    can_stop: delta.can_stop ?? false,
  }
}

function applyDelta(stored: WorkflowRun, delta: WorkflowDelta): WorkflowRun {
  const next = { ...stored }
  if (delta.name != null) next.name = delta.name
  if (delta.objective != null) next.objective = delta.objective
  if (delta.state != null) next.state = delta.state
  if (delta.phases != null) next.phases = delta.phases
  if (delta.current_phase != null) next.current_phase = delta.current_phase
  if (delta.agents != null) next.agents = delta.agents
  if (delta.agents_done != null) next.agents_done = delta.agents_done
  if (delta.agents_running != null) next.agents_running = delta.agents_running
  if (delta.agents_used != null) next.agents_used = delta.agents_used
  if (delta.agent_budget != null) next.agent_budget = delta.agent_budget
  if (delta.agents_remaining != null)
    next.agents_remaining = delta.agents_remaining
  if (delta.elapsed_ms != null) next.elapsed_ms = delta.elapsed_ms
  if (delta.last_event != null) next.last_event = delta.last_event
  if (delta.last_event_detail != null)
    next.last_event_detail = delta.last_event_detail
  if (delta.can_stop != null) next.can_stop = delta.can_stop
  return next
}

export function upsertWorkflow(
  current: WorkflowRun[],
  delta: WorkflowDelta
): WorkflowRun[] {
  if (!delta?.run_id) return current
  const index = current.findIndex((r) => r.run_id === delta.run_id)
  if (index < 0) {
    if (!delta.spawned) return current
    return [...current, recordFromDelta(delta)]
  }
  const next = [...current]
  next[index] = applyDelta(current[index], delta)
  return next
}

export function mergeWorkflows(
  current: WorkflowRun[],
  incoming: WorkflowRun[] | null | undefined
): WorkflowRun[] {
  if (!incoming || incoming.length === 0) return current
  let next: WorkflowRun[] | null = null
  for (const record of incoming) {
    if (!record?.run_id) continue
    const target = next ?? current
    const index = target.findIndex((r) => r.run_id === record.run_id)
    if (index >= 0) {
      next ??= [...current]
      next[index] = record
    } else {
      next ??= [...current]
      next.push(record)
    }
  }
  return next ?? current
}

export function adoptUnknownWorkflows(
  current: WorkflowRun[],
  incoming: WorkflowRun[] | null | undefined
): WorkflowRun[] {
  if (!incoming || incoming.length === 0) return current
  let next: WorkflowRun[] | null = null
  for (const record of incoming) {
    if (!record?.run_id) continue
    const target = next ?? current
    if (target.some((r) => r.run_id === record.run_id)) continue
    next ??= [...current]
    next.push(record)
  }
  return next ?? current
}

export function liveWorkflows(runs: WorkflowRun[]): WorkflowRun[] {
  return runs.filter((r) => !isWorkflowTerminal(r))
}

export function phaseProgress(run: WorkflowRun): {
  current: number
  total: number
} | null {
  if (!run.phases.length) return null
  const idx = run.current_phase
    ? run.phases.findIndex((p) => p.title === run.current_phase)
    : run.phases.findIndex((p) => p.state === "active")
  return {
    current: idx >= 0 ? idx + 1 : 0,
    total: run.phases.length,
  }
}

export interface WorkflowPhaseGroup {
  phase: WorkflowPhase | null
  agents: WorkflowAgent[]
}

/** Group child agents under their phase. Agents with no matching phase land
 *  in a trailing untitled group. */
export function groupAgentsByPhase(run: WorkflowRun): WorkflowPhaseGroup[] {
  const agents = run.agents ?? []
  if (run.phases.length === 0) {
    return agents.length > 0 ? [{ phase: null, agents }] : []
  }
  const used = new Set<string>()
  const groups: WorkflowPhaseGroup[] = run.phases.map((phase) => {
    const nodes = agents.filter((a) => a.phase === phase.title)
    for (const n of nodes) used.add(n.agent_id)
    return { phase, agents: nodes }
  })
  const leftover = agents.filter((a) => !used.has(a.agent_id))
  if (leftover.length > 0) {
    groups.push({ phase: null, agents: leftover })
  }
  return groups
}
