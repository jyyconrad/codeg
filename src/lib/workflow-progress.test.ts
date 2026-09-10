import { describe, expect, it } from "vitest"

import {
  adoptUnknownWorkflows,
  liveWorkflows,
  phaseProgress,
  upsertWorkflow,
} from "./workflow-progress"
import type { WorkflowDelta, WorkflowRun } from "@/lib/types"

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    run_id: "wf_1",
    name: "deep-research",
    objective: "survey",
    state: "running",
    phases: [
      { title: "Plan", state: "active" },
      { title: "Research", state: "pending" },
    ],
    current_phase: "Plan",
    agents_done: 0,
    agents_running: 1,
    agents_used: 1,
    elapsed_ms: 23_000,
    last_event: "Plan",
    can_stop: false,
    ...overrides,
  }
}

function delta(overrides: Partial<WorkflowDelta> = {}): WorkflowDelta {
  return { run_id: "wf_1", spawned: true, ...overrides }
}

describe("upsertWorkflow", () => {
  it("creates a row only from a spawned delta", () => {
    expect(upsertWorkflow([], delta({ spawned: false }))).toEqual([])
    const created = upsertWorkflow([], delta({ name: "deep-research" }))
    expect(created).toHaveLength(1)
    expect(created[0].name).toBe("deep-research")
    expect(created[0].state).toBe("running")
  })

  it("patches present fields and leaves the rest", () => {
    const [next] = upsertWorkflow(
      [run()],
      delta({ spawned: false, current_phase: "Research", agents_done: 1 })
    )
    expect(next.current_phase).toBe("Research")
    expect(next.agents_done).toBe(1)
    expect(next.agents_running).toBe(1)
    expect(next.name).toBe("deep-research")
  })
})

describe("liveWorkflows", () => {
  it("drops terminal runs", () => {
    expect(
      liveWorkflows([
        run({ state: "completed" }),
        run({ run_id: "wf_2", state: "running" }),
      ]).map((r) => r.run_id)
    ).toEqual(["wf_2"])
  })
})

describe("phaseProgress", () => {
  it("indexes the current phase", () => {
    expect(phaseProgress(run({ current_phase: "Research" }))).toEqual({
      current: 2,
      total: 2,
    })
  })

  it("returns null when the run has no phase rail", () => {
    expect(phaseProgress(run({ phases: [] }))).toBeNull()
  })
})

describe("adoptUnknownWorkflows", () => {
  it("adds unseen ids without clobbering existing rows", () => {
    const current = [run({ name: "keep" })]
    const merged = adoptUnknownWorkflows(current, [
      run({ name: "stale" }),
      run({ run_id: "wf_new", name: "new" }),
    ])
    expect(merged.map((r) => r.name)).toEqual(["keep", "new"])
  })
})
