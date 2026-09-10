import { render, screen } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { describe, expect, it } from "vitest"

import { WorkflowProgressStrip } from "./workflow-progress-strip"
import enMessages from "@/i18n/messages/en.json"
import type { WorkflowRun } from "@/lib/types"

function run(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    run_id: "wf_1",
    name: "deep-research",
    objective: "survey sensors",
    state: "running",
    phases: [
      { title: "Plan", state: "done" },
      { title: "Research", state: "active" },
      { title: "Verify", state: "pending" },
      { title: "Report", state: "pending" },
    ],
    current_phase: "Research",
    agents_done: 1,
    agents_running: 4,
    agents_used: 5,
    elapsed_ms: 43_000,
    last_event: "Research",
    can_stop: false,
    ...overrides,
  }
}

function renderStrip(runs: WorkflowRun[]) {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <WorkflowProgressStrip runs={runs} />
    </NextIntlClientProvider>
  )
}

describe("WorkflowProgressStrip", () => {
  it("renders nothing once every run has settled", () => {
    const { container } = renderStrip([run({ state: "completed" })])
    expect(container).toBeEmptyDOMElement()
  })

  it("shows name, phase rail, agent counts, and elapsed without dumping content", () => {
    renderStrip([run()])
    expect(screen.getByText("deep-research")).toBeInTheDocument()
    expect(screen.getByText("Plan")).toBeInTheDocument()
    expect(screen.getByText("Research")).toBeInTheDocument()
    expect(screen.getByText("Verify")).toBeInTheDocument()
    expect(screen.getByText("Report")).toBeInTheDocument()
    expect(screen.getByText(/1 done/)).toBeInTheDocument()
    expect(screen.getByText(/4 running/)).toBeInTheDocument()
    expect(screen.queryByText(/survey sensors/)).not.toBeInTheDocument()
  })
})
