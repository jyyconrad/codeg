import { describe, expect, it, beforeEach } from "vitest"

import {
  resetWorkflowClocks,
  syncWorkflowClock,
  workflowElapsedMs,
} from "./workflow-clock"

describe("workflowClock", () => {
  beforeEach(() => {
    resetWorkflowClocks()
  })

  it("starts at zero when a running row is first seen", () => {
    expect(syncWorkflowClock("wf", "running", 1_000)).toBe(0)
  })

  it("accumulates while running and freezes on pause", () => {
    syncWorkflowClock("wf", "running", 1_000)
    expect(syncWorkflowClock("wf", "running", 4_000)).toBe(3_000)
    expect(syncWorkflowClock("wf", "paused", 5_000)).toBe(4_000)
    expect(workflowElapsedMs("wf", 9_000)).toBe(4_000)
  })

  it("resumes from the paused mark and freezes when the run settles", () => {
    syncWorkflowClock("wf", "running", 1_000)
    syncWorkflowClock("wf", "paused", 3_000)
    syncWorkflowClock("wf", "running", 10_000)
    expect(syncWorkflowClock("wf", "running", 12_000)).toBe(4_000)
    expect(syncWorkflowClock("wf", "completed", 13_000)).toBe(5_000)
    expect(workflowElapsedMs("wf", 20_000)).toBe(5_000)
  })

  it("does not start a clock for a terminal first frame", () => {
    expect(syncWorkflowClock("wf", "failed", 1_000)).toBe(0)
    expect(workflowElapsedMs("wf", 8_000)).toBe(0)
  })
})
