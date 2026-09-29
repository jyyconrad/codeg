import { describe, expect, it } from "vitest"

import {
  isWorkflowFollowUpMessage,
  workflowFollowUpRunId,
} from "./workflow-follow-up-message"

const FOLLOW_UP_TEXT = [
  "[Codeg workflow completion]",
  "This is a host-generated event, not a new user request.",
  "Workflow: build",
  "Run ID: run-123",
  "Status: completed",
].join("\n")

describe("workflowFollowUpRunId", () => {
  it("reads a run id from the message id prefix", () => {
    expect(
      workflowFollowUpRunId("codeg-workflow-follow-up:run-123", null)
    ).toBe("run-123")
    expect(
      workflowFollowUpRunId(
        "codeg-workflow-follow-up:  run-123  ",
        "Run ID: other"
      )
    ).toBe("run-123")
    expect(
      isWorkflowFollowUpMessage("codeg-workflow-follow-up:run-123", "hello")
    ).toBe(true)
  })

  it("reads a run id from the completion mark and Run ID line", () => {
    expect(workflowFollowUpRunId(null, FOLLOW_UP_TEXT)).toBe("run-123")
    expect(workflowFollowUpRunId(undefined, FOLLOW_UP_TEXT)).toBe("run-123")
    expect(isWorkflowFollowUpMessage(undefined, FOLLOW_UP_TEXT)).toBe(true)
  })

  it("is false for ordinary user text", () => {
    expect(
      isWorkflowFollowUpMessage("optimistic-1", "please fix the test")
    ).toBe(false)
    expect(workflowFollowUpRunId("msg-1", "please fix the test")).toBeNull()
  })

  it("is false when the mark is not the first line", () => {
    const text = [
      "Result body",
      "[Codeg workflow completion]",
      "Run ID: run-123",
    ].join("\n")
    expect(isWorkflowFollowUpMessage("turn-2", text)).toBe(false)
    expect(workflowFollowUpRunId(null, text)).toBeNull()
  })

  it("recognizes the structured status prompt without a result body", () => {
    const text = [
      "[system notification]",
      "Workflow: deep-research",
      "Status: completed",
      "",
      "This run has ended. Use the outcome already in this conversation. Compare it with the user's goal and do the next required step in this turn.",
      "- If the goal is met, summarize what was completed, then stop.",
      "- If work remains, continue that work now. Do not stop at a proposal.",
      "Do not restart this workflow merely because this notification arrived.",
    ].join("\n")
    expect(isWorkflowFollowUpMessage("grok-turn-2", text)).toBe(true)
    expect(workflowFollowUpRunId("grok-turn-2", text)).toBeNull()
    expect(
      isWorkflowFollowUpMessage(
        "optimistic-1",
        "[system notification]\nplease summarize"
      )
    ).toBe(false)
  })

  it("is false when the prefix remainder is empty", () => {
    expect(workflowFollowUpRunId("codeg-workflow-follow-up:", null)).toBeNull()
    expect(
      isWorkflowFollowUpMessage("codeg-workflow-follow-up:   ", "hello")
    ).toBe(false)
  })
})
