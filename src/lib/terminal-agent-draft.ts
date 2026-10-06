import { draftRidesBlocks, getPromptDraftDisplayText } from "@/lib/prompt-draft"
import type { PromptDraft } from "@/lib/types"

export function terminalAgentPrompt(
  draft: PromptDraft,
  attachedResourcesFallback: string
): string | null {
  if (draftRidesBlocks(draft)) return null
  return getPromptDraftDisplayText(draft, attachedResourcesFallback)
}
