/**
 * Client-owned elapsed time for a live workflow run.
 *
 * Grok's `elapsed_ms` on `workflow_updated` is not a reliable wall clock
 * (captures often stay at 0). The panel starts counting when codeg first
 * sees the run as `running`, pauses with the run, and freezes on a
 * terminal state. Not a server fact — UI-only.
 */

const RUNNING = "running"

type Clock = {
  startedAt: number | null
  accumulatedMs: number
}

const clocks = new Map<string, Clock>()

export function isWorkflowClockRunning(state: string): boolean {
  return state === RUNNING
}

export function resetWorkflowClocks(): void {
  clocks.clear()
}

/** Apply `state` at `now` and return elapsed milliseconds. */
export function syncWorkflowClock(
  runId: string,
  state: string,
  now: number = Date.now()
): number {
  const running = isWorkflowClockRunning(state)
  let clock = clocks.get(runId)
  if (!clock) {
    clock = {
      startedAt: running ? now : null,
      accumulatedMs: 0,
    }
    clocks.set(runId, clock)
    return 0
  }
  if (running && clock.startedAt == null) {
    clock.startedAt = now
  } else if (!running && clock.startedAt != null) {
    clock.accumulatedMs += Math.max(0, now - clock.startedAt)
    clock.startedAt = null
  }
  return elapsedFrom(clock, now)
}

export function workflowElapsedMs(
  runId: string,
  now: number = Date.now()
): number {
  const clock = clocks.get(runId)
  if (!clock) return 0
  return elapsedFrom(clock, now)
}

function elapsedFrom(clock: Clock, now: number): number {
  const live = clock.startedAt != null ? Math.max(0, now - clock.startedAt) : 0
  return clock.accumulatedMs + live
}
