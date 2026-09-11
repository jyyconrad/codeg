import { act, fireEvent, render, screen } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { WorkflowProgressDockProvider } from "./workflow-progress-dock"
import { WorkflowProgressOverlay } from "./workflow-progress-overlay"
import enMessages from "@/i18n/messages/en.json"
import { resetWorkflowClocks } from "@/lib/workflow-clock"
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
    agents: [
      {
        agent_id: "a1",
        label: "research-planner",
        phase: "Plan",
        state: "done",
        tokens_used: 22284,
        duration_ms: 43_000,
      },
      {
        agent_id: "a2",
        label: "researcher-0",
        phase: "Research",
        state: "running",
        tokens_used: 1200,
      },
    ],
    agents_done: 1,
    agents_running: 1,
    agents_used: 2,
    agent_budget: 128,
    elapsed_ms: 43_000,
    last_event: "phase_entered",
    last_event_detail: "Research",
    can_stop: false,
    ...overrides,
  }
}

function renderOverlay(
  runs: WorkflowRun[],
  placement: "overlay" | "composer" = "overlay"
) {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <WorkflowProgressDockProvider runs={runs}>
        <WorkflowProgressOverlay placement={placement} />
      </WorkflowProgressDockProvider>
    </NextIntlClientProvider>
  )
}

describe("WorkflowProgressOverlay", () => {
  beforeEach(() => {
    window.localStorage.removeItem("codeg:workflow-progress-dock")
    resetWorkflowClocks()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it("renders nothing once every run has settled", () => {
    const { container } = renderOverlay([run({ state: "completed" })])
    expect(container).toBeEmptyDOMElement()
  })

  it("renders nothing for the composer placement while docked as overlay", () => {
    const { container } = renderOverlay([run()], "composer")
    expect(container).toBeEmptyDOMElement()
  })

  it("shows a vertical plan-style list of phases and nodes", () => {
    renderOverlay([run()])
    expect(screen.getByText("Workflow")).toBeInTheDocument()
    expect(screen.getByText("2/4")).toBeInTheDocument()
    expect(screen.getByText("Plan")).toBeInTheDocument()
    expect(screen.getByText("Research")).toBeInTheDocument()
    expect(screen.getByText("research-planner")).toBeInTheDocument()
    expect(screen.getByText("researcher-0")).toBeInTheDocument()
    expect(screen.queryByText("survey sensors")).not.toBeInTheDocument()
    expect(screen.queryByText("Active")).not.toBeInTheDocument()
    expect(screen.queryByText("Done")).not.toBeInTheDocument()
    expect(screen.queryByText("Running")).not.toBeInTheDocument()
    expect(screen.queryByText("Pending")).not.toBeInTheDocument()
  })

  it("prefers phase detail and node summary as the single text line", () => {
    renderOverlay([
      run({
        phases: [
          {
            title: "Survey",
            detail: "read-only gap analysis vs the design",
            state: "active",
          },
        ],
        current_phase: "Survey",
        agents: [
          {
            agent_id: "a1",
            label: "survey:host",
            phase: "Survey",
            state: "running",
            summary: "Map host types and registry",
          },
        ],
      }),
    ])
    expect(
      screen.getByText("read-only gap analysis vs the design")
    ).toBeInTheDocument()
    expect(screen.getByText("Map host types and registry")).toBeInTheDocument()
    expect(screen.queryByText("Survey")).not.toBeInTheDocument()
    expect(screen.queryByText("survey:host")).not.toBeInTheDocument()
  })

  it("collapses the overlay to the icon chip", () => {
    renderOverlay([run()])
    fireEvent.click(screen.getByRole("button", { name: "Collapse workflow" }))
    expect(screen.queryByText("research-planner")).not.toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /deep-research 2\/4/ })
    ).toBeInTheDocument()
  })

  it("docks above the input at full width and collapses to name, progress, elapsed", () => {
    render(
      <NextIntlClientProvider locale="en" messages={enMessages}>
        <WorkflowProgressDockProvider runs={[run()]}>
          <WorkflowProgressOverlay placement="overlay" />
          <WorkflowProgressOverlay placement="composer" />
        </WorkflowProgressDockProvider>
      </NextIntlClientProvider>
    )
    fireEvent.click(screen.getByLabelText("Dock above the input"))
    expect(screen.getByText("deep-research")).toBeInTheDocument()
    expect(screen.getByText("2/4")).toBeInTheDocument()
    expect(screen.getByText("0s")).toBeInTheDocument()
    expect(screen.getByText("research-planner")).toBeInTheDocument()

    fireEvent.click(screen.getByRole("button", { name: "Collapse workflow" }))
    expect(screen.queryByText("research-planner")).not.toBeInTheDocument()
    expect(screen.getByText("deep-research")).toBeInTheDocument()
    expect(screen.getByText("2/4")).toBeInTheDocument()
    expect(screen.getByText("0s")).toBeInTheDocument()
  })

  it("ticks a local clock while the run is live", () => {
    vi.useFakeTimers()
    render(
      <NextIntlClientProvider locale="en" messages={enMessages}>
        <WorkflowProgressDockProvider runs={[run()]}>
          <WorkflowProgressOverlay placement="overlay" />
          <WorkflowProgressOverlay placement="composer" />
        </WorkflowProgressDockProvider>
      </NextIntlClientProvider>
    )
    fireEvent.click(screen.getByLabelText("Dock above the input"))
    expect(screen.getByText("0s")).toBeInTheDocument()
    act(() => {
      vi.advanceTimersByTime(3000)
    })
    expect(screen.getByText("3s")).toBeInTheDocument()
  })
})
