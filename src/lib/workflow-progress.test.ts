import { describe, expect, it } from "vitest"

import {
  adoptUnknownWorkflows,
  groupAgentsByPhase,
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
    agents: [],
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

  it("replaces the agent node list when the delta carries one", () => {
    const [next] = upsertWorkflow(
      [run()],
      delta({
        spawned: false,
        agents: [
          {
            agent_id: "a1",
            label: "planner",
            phase: "Plan",
            state: "done",
          },
        ],
      })
    )
    expect(next.agents).toHaveLength(1)
    expect(next.agents[0].label).toBe("planner")
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

describe("groupAgentsByPhase", () => {
  it("nests nodes under the matching phase and keeps leftovers", () => {
    const grouped = groupAgentsByPhase(
      run({
        agents: [
          {
            agent_id: "a1",
            label: "planner",
            phase: "Plan",
            state: "done",
          },
          {
            agent_id: "a2",
            label: "researcher",
            phase: "Research",
            state: "running",
          },
          {
            agent_id: "a3",
            label: "orphan",
            phase: "Other",
            state: "pending",
          },
        ],
      })
    )
    expect(
      grouped.map((g) => [g.phase?.title ?? null, g.agents.map((a) => a.label)])
    ).toEqual([
      ["Plan", ["planner"]],
      ["Research", ["researcher"]],
      [null, ["orphan"]],
    ])
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
