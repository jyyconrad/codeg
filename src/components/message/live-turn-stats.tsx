"use client"

import { useEffect, useMemo, useState } from "react"
import { useLocale, useTranslations } from "next-intl"
import type {
  LiveContentBlock,
  LiveMessage,
} from "@/contexts/acp-connections-context"
import { inferLiveToolName } from "@/lib/tool-call-normalization"
import { formatElapsedLabel } from "@/lib/format-elapsed"
import { extractReplyFileChanges } from "@/lib/session-files"
import { formatTokenCount } from "@/lib/token-format"
import { Coins, FilePenLine, Plane, Timer } from "lucide-react"
import type { AgentType, TurnUsage } from "@/lib/types"
import { AgentIcon } from "@/components/agent-icon"
import { useTokenOutputSpeed } from "@/hooks/use-token-output-speed"
import { useConversationRuntimeStore } from "@/stores/conversation-runtime-store"

interface LiveTurnStatsProps {
  message: LiveMessage
  agentType: AgentType
  conversationId: number
  isStreaming?: boolean
}

interface LineChangeStats {
  additions: number
  deletions: number
}

interface LiveEditStats extends LineChangeStats {
  files: number
}

function formatCompactInt(n: number, formatter: Intl.NumberFormat): string {
  if (n < 1000) return String(n)
  return formatter.format(n)
}

interface BlockEditContribution {
  files: string[]
  additions: number
  deletions: number
}

const WRITE_OPS = new Set(["edit", "write", "apply_patch"])

// Parsing a tool call's `raw_input` (JSON.parse + diff line counting) is the
// expensive part of the live edit stats, and it re-runs on every streaming
// token because the `message` reference changes each token. A completed
// tool_call block keeps a stable reference across tokens (the reducer rebuilds
// only the block that changed — see acp-connections-context), so cache each
// block's contribution keyed on the block object. Only the block currently
// being updated re-parses; the rest are O(1) lookups. Keying on the block ref
// is sound because a block's ref changes iff its content changes, and the
// WeakMap lets dropped blocks be collected.
const blockEditContributionCache = new WeakMap<
  LiveContentBlock,
  BlockEditContribution | null
>()

function computeBlockEditContribution(
  block: LiveContentBlock
): BlockEditContribution | null {
  if (block.type !== "tool_call") return null
  const toolName = inferLiveToolName({
    title: block.info.title,
    kind: block.info.kind,
    rawInput: block.info.raw_input,
    meta: block.info.meta,
  })
  if (!WRITE_OPS.has(toolName)) return null

  const files = extractReplyFileChanges([
    {
      id: block.info.tool_call_id,
      role: "assistant",
      timestamp: "",
      blocks: [
        {
          type: "tool_use",
          tool_use_id: block.info.tool_call_id,
          tool_name: toolName,
          input_preview: block.info.raw_input,
        },
      ],
    },
  ])
  if (files.length === 0) return null

  return {
    files: files.map((file) => file.path),
    additions: files.reduce((sum, file) => sum + file.additions, 0),
    deletions: files.reduce((sum, file) => sum + file.deletions, 0),
  }
}

function blockEditContribution(
  block: LiveContentBlock
): BlockEditContribution | null {
  const cached = blockEditContributionCache.get(block)
  if (cached !== undefined) return cached
  const contribution = computeBlockEditContribution(block)
  blockEditContributionCache.set(block, contribution)
  return contribution
}

export function extractLiveEditStats(message: LiveMessage): LiveEditStats {
  const files = new Set<string>()
  let additions = 0
  let deletions = 0

  for (const block of message.content) {
    const contribution = blockEditContribution(block)
    if (!contribution) continue
    for (const path of contribution.files) files.add(path)
    additions += contribution.additions
    deletions += contribution.deletions
  }

  return { files: files.size, additions, deletions }
}

function sessionCacheTokens(usage: TurnUsage): number {
  return usage.cache_read_input_tokens + usage.cache_creation_input_tokens
}

export function LiveTurnStats({
  message,
  agentType,
  conversationId,
  isStreaming = true,
}: LiveTurnStatsProps) {
  const locale = useLocale()
  const t = useTranslations("Folder.chat.liveTurnStats")
  const [elapsed, setElapsed] = useState(() => Date.now() - message.startedAt)
  const editStats = useMemo(() => extractLiveEditStats(message), [message])
  const tps = useTokenOutputSpeed(message)
  const usage = useConversationRuntimeStore(
    (s) =>
      s.byConversationId.get(conversationId)?.sessionStats?.total_usage ?? null
  )
  const compactNumberFormatter = useMemo(
    () =>
      new Intl.NumberFormat(locale, {
        notation: "compact",
        maximumFractionDigits: 1,
      }),
    [locale]
  )

  useEffect(() => {
    const timer = setInterval(() => {
      setElapsed(Date.now() - message.startedAt)
    }, 1_000)
    return () => clearInterval(timer)
  }, [message.startedAt])

  const hasThinkingBlock = message.content.some((b) => b.type === "thinking")

  // Only active streams should show thinking/streaming state.
  const lastBlock = message.content[message.content.length - 1]
  const isThinking =
    isStreaming &&
    hasThinkingBlock &&
    message.content.length <= 1 &&
    lastBlock?.type === "thinking"

  const elapsedLabel = formatElapsedLabel(elapsed, t)
  const cacheTokens = usage ? sessionCacheTokens(usage) : 0
  const hasUsage =
    usage != null &&
    (usage.input_tokens > 0 || usage.output_tokens > 0 || cacheTokens > 0)
  const tokenTooltip = usage
    ? [
        `${t("tokenInput")} ${formatTokenCount(usage.input_tokens)}`,
        `${t("tokenOutput")} ${formatTokenCount(usage.output_tokens)}`,
        `${t("tokenCache")} ${formatTokenCount(cacheTokens)}`,
      ].join(" · ")
    : undefined

  return (
    <div className="@container/turnstats shrink-0">
      <div className="flex min-h-8 flex-wrap items-center justify-center gap-x-3 gap-y-1 px-4 py-1 text-xs leading-none text-muted-foreground">
        <AgentIcon
          agentType={agentType}
          className="h-3.5 w-3.5 animate-pulse"
        />
        {isThinking ? (
          <span>{t("thinking")}</span>
        ) : (
          <span>{t("streaming")}</span>
        )}
        <span className="text-border leading-none">|</span>
        <span className="inline-flex items-center gap-1 leading-none">
          <Timer className="h-3 w-3 shrink-0" />
          {elapsedLabel}
        </span>
        {editStats.files > 0 && (
          <>
            <span className="hidden text-border leading-none @[24rem]/turnstats:inline">
              |
            </span>
            <span className="hidden items-center gap-1 leading-none @[24rem]/turnstats:inline-flex">
              <FilePenLine className="h-3 w-3 shrink-0" />
              {editStats.files}F +
              {formatCompactInt(editStats.additions, compactNumberFormatter)}/-
              {formatCompactInt(editStats.deletions, compactNumberFormatter)}
            </span>
          </>
        )}
        {hasUsage && usage && (
          <>
            <span className="hidden text-border leading-none @[28rem]/turnstats:inline">
              |
            </span>
            <span
              className="hidden items-center gap-1 leading-none tabular-nums @[28rem]/turnstats:inline-flex"
              title={tokenTooltip}
            >
              <Coins
                aria-label={t("tokenUsageAria")}
                className="h-3 w-3 shrink-0"
              />
              <span>↑{formatTokenCount(usage.input_tokens)}</span>
              <span>↓{formatTokenCount(usage.output_tokens)}</span>
              <span>⚡{formatTokenCount(cacheTokens)}</span>
            </span>
          </>
        )}
        {tps != null && (
          <>
            <span className="hidden text-border leading-none @[36rem]/turnstats:inline">
              |
            </span>
            {/* `tabular-nums` so the digits stop shimmering as the rate moves. */}
            <span
              className="hidden items-center gap-1 leading-none tabular-nums @[36rem]/turnstats:inline-flex"
              title={t("outputSpeedTooltip")}
            >
              {/* Name hangs off the icon, matching `ComposerContextUsage` —
                  `aria-label` on the bare wrapper span carries no role and
                  isn't reliably exposed. */}
              <Plane
                aria-label={t("outputSpeedAria")}
                className="h-3 w-3 shrink-0"
              />
              {tps.toFixed(1)} tok/s
            </span>
          </>
        )}
      </div>
    </div>
  )
}
