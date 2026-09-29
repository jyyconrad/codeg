export const WORKFLOW_FOLLOW_UP_ID_PREFIX = "codeg-workflow-follow-up:"
export const WORKFLOW_FOLLOW_UP_TEXT_MARK = "[Codeg workflow completion]"

const RUN_ID_LINE = /^Run ID: (\S+)$/

function withoutCarriageReturn(line: string): string {
  return line.endsWith("\r") ? line.slice(0, -1) : line
}

/** Run id of a host workflow follow-up, or null when this is a human prompt.
 *
 * The live broadcast id wins. Text is the fallback for a transcript that
 * still carries the fixed template but not the id. */
export function workflowFollowUpRunId(
  messageId: string | null | undefined,
  text: string | null | undefined
): string | null {
  if (
    typeof messageId === "string" &&
    messageId.startsWith(WORKFLOW_FOLLOW_UP_ID_PREFIX)
  ) {
    const runId = messageId.slice(WORKFLOW_FOLLOW_UP_ID_PREFIX.length).trim()
    if (runId.length > 0) return runId
  }
  if (typeof text !== "string" || text.length === 0) return null
  const newline = text.indexOf("\n")
  const firstLine = withoutCarriageReturn(
    newline === -1 ? text : text.slice(0, newline)
  )
  if (firstLine !== WORKFLOW_FOLLOW_UP_TEXT_MARK) return null
  const rest = newline === -1 ? "" : text.slice(newline + 1)
  for (const raw of rest.split("\n")) {
    const match = RUN_ID_LINE.exec(withoutCarriageReturn(raw))
    if (match?.[1]) return match[1]
  }
  return null
}

const LEGACY_OPEN = "[system notification] workflow "
const LEGACY_DECISION =
  "。根据执行情况和任务目标，决定下一步应该怎么工作，例如总结完成情况、推进下一步工作。"
const STRUCTURED_OPEN = "[system notification]\nWorkflow: "
const STRUCTURED_LEAD =
  "This run has ended. Use the outcome already in this conversation. Compare it with the user's goal and do the next required step in this turn.\n"
const STRUCTURED_CLOSE =
  "Do not restart this workflow merely because this notification arrived."

function isLegacyDecisionSentence(body: string): boolean {
  if (!body.startsWith(LEGACY_OPEN) || !body.endsWith(LEGACY_DECISION)) {
    return false
  }
  const middle = body.slice(
    LEGACY_OPEN.length,
    body.length - LEGACY_DECISION.length
  )
  const statusAt = middle.lastIndexOf(" 已经")
  if (statusAt <= 0) return false
  const status = middle.slice(statusAt)
  return status === " 已经完成" || status === " 已经失败"
}

function isStructuredDecision(body: string): boolean {
  if (!body.startsWith(STRUCTURED_OPEN) || !body.endsWith(STRUCTURED_CLOSE)) {
    return false
  }
  const rest = body.slice(STRUCTURED_OPEN.length)
  const nameEnd = rest.indexOf("\n")
  if (nameEnd <= 0) return false
  const afterName = rest.slice(nameEnd + 1)
  const completed =
    "Status: completed\n\n" +
    STRUCTURED_LEAD +
    "- If the goal is met, summarize what was completed, then stop.\n" +
    "- If work remains, continue that work now. Do not stop at a proposal.\n" +
    STRUCTURED_CLOSE
  const failed =
    "Status: failed\n\n" +
    STRUCTURED_LEAD +
    "- State why this run failed.\n" +
    "- Take the next justified step now. If the goal cannot continue, summarize the blocker and stop.\n" +
    STRUCTURED_CLOSE
  return afterName === completed || afterName === failed
}

/** The short host prompt. Older transcripts still use the completion mark. */
function isWorkflowDecisionSentence(text: string | null | undefined): boolean {
  if (typeof text !== "string") return false
  const body = text.endsWith("\n") ? text.slice(0, -1) : text
  return isLegacyDecisionSentence(body) || isStructuredDecision(body)
}

export function isWorkflowFollowUpMessage(
  messageId: string | null | undefined,
  text: string | null | undefined
): boolean {
  return (
    workflowFollowUpRunId(messageId, text) !== null ||
    isWorkflowDecisionSentence(text)
  )
}
