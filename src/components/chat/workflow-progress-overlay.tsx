"use client"

/**
 * Live workflow progress with two placements, matching neighbouring chrome:
 *
 * - Overlay stack (default): same w-72 / max-h-96 card as `AgentPlanOverlay`,
 *   stacked beneath it. Collapsed = icon-only chip. Body is a vertical list of
 *   phase/node rows, like the plan entries.
 * - Composer dock: same max-w-3xl + px-4 column as the input. Collapsed = one
 *   title row `{name} {progress} {elapsed}`. Expanded body lays phases and
 *   nodes out horizontally.
 */

import { useState } from "react"
import { useTranslations } from "next-intl"
import {
  AlertCircle,
  CheckCircle2Icon,
  ChevronDownIcon,
  CircleDashedIcon,
  Loader2Icon,
  PanelBottom,
  PanelLeft,
  Workflow,
} from "lucide-react"

import { CollapsedOverlayChip } from "@/components/chat/collapsed-overlay-chip"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import { formatElapsedLabel } from "@/lib/format-elapsed"
import { formatTokenCount } from "@/lib/token-format"
import {
  groupAgentsByPhase,
  liveWorkflows,
  phaseProgress,
} from "@/lib/workflow-progress"
import type { WorkflowAgent, WorkflowPhase, WorkflowRun } from "@/lib/types"
import {
  useWorkflowProgressDock,
  type WorkflowProgressDock,
} from "@/components/chat/workflow-progress-dock"

export function WorkflowProgressOverlay({
  placement,
}: {
  placement: WorkflowProgressDock
}) {
  const ctx = useWorkflowProgressDock()
  if (!ctx || ctx.dock !== placement) return null
  const live = liveWorkflows(ctx.runs)
  if (live.length === 0) return null
  return (
    <WorkflowProgressPanel
      runs={live}
      placement={placement}
      onToggleDock={ctx.toggleDock}
    />
  )
}

function WorkflowProgressPanel({
  runs,
  placement,
  onToggleDock,
}: {
  runs: WorkflowRun[]
  placement: WorkflowProgressDock
  onToggleDock: () => void
}) {
  const t = useTranslations("Folder.chat.workflows")
  const tElapsed = useTranslations("Folder.chat.liveTurnStats")
  const overlay = placement === "overlay"
  const key = runs.map((r) => r.run_id).join("|")
  const [collapsedByKey, setCollapsedByKey] = useState<Record<string, boolean>>(
    {}
  )
  const userCollapsed = collapsedByKey[key]
  const isExpanded = userCollapsed !== undefined ? !userCollapsed : true
  const primary = runs[0]
  const progress = phaseProgress(primary)
  const elapsed =
    typeof primary.elapsed_ms === "number" && primary.elapsed_ms > 0
      ? formatElapsedLabel(primary.elapsed_ms, tElapsed)
      : null
  const progressLabel = progress
    ? `${progress.current}/${progress.total}`
    : null
  const expand = () => setCollapsedByKey((prev) => ({ ...prev, [key]: false }))
  const collapse = () => setCollapsedByKey((prev) => ({ ...prev, [key]: true }))

  if (!isExpanded && overlay) {
    const summary = progress
      ? t("collapsedSummary", {
          name: primary.name,
          current: progress.current,
          total: progress.total,
        })
      : t("collapsedSummaryNamed", { name: primary.name })
    return (
      <CollapsedOverlayChip
        icon={<Workflow className="size-3" />}
        summary={summary}
        onClick={expand}
      />
    )
  }

  const header = (
    <div className="flex items-center justify-between gap-1 border-b px-3 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <Workflow className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="truncate text-sm font-medium">
          {overlay ? t("title") : primary.name}
        </span>
        {progressLabel ? (
          <Badge variant="secondary" className="h-5">
            {progressLabel}
          </Badge>
        ) : runs.length > 1 ? (
          <Badge variant="secondary" className="h-5">
            {runs.length}
          </Badge>
        ) : null}
        {!overlay && elapsed ? (
          <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
            {elapsed}
          </span>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center">
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={
            overlay ? t("dockToComposerAria") : t("dockToOverlayAria")
          }
          onClick={(e) => {
            e.stopPropagation()
            onToggleDock()
          }}
        >
          {overlay ? (
            <PanelBottom className="h-4 w-4" />
          ) : (
            <PanelLeft className="h-4 w-4" />
          )}
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          aria-label={isExpanded ? t("collapseAria") : t("expandAria")}
          onClick={(e) => {
            e.stopPropagation()
            if (isExpanded) collapse()
            else expand()
          }}
        >
          <ChevronDownIcon
            className={cn("h-4 w-4", !isExpanded && "rotate-180")}
          />
        </Button>
      </div>
    </div>
  )

  if (!isExpanded) {
    return (
      <div
        className="mb-2 w-full rounded-xl border bg-card/60 shadow-lg backdrop-blur transition-colors hover:bg-card/95 supports-[backdrop-filter]:bg-card/50 supports-[backdrop-filter]:hover:bg-card/85"
        data-testid="workflow-composer-collapsed"
      >
        {header}
      </div>
    )
  }

  const body = (
    <div
      className={cn(
        "p-3",
        overlay
          ? "max-h-96 space-y-2 overflow-y-auto"
          : "flex gap-2 overflow-x-auto"
      )}
    >
      {runs.map((run) => (
        <WorkflowRunBody
          key={run.run_id}
          run={run}
          layout={overlay ? "vertical" : "horizontal"}
        />
      ))}
    </div>
  )

  if (overlay) {
    return (
      <div className="pointer-events-none flex max-w-[min(22rem,calc(100%-2rem))]">
        <div className="pointer-events-auto w-72 max-w-full rounded-xl border bg-card/60 shadow-lg backdrop-blur transition-colors hover:bg-card/95 supports-[backdrop-filter]:bg-card/50 supports-[backdrop-filter]:hover:bg-card/85">
          {header}
          {body}
        </div>
      </div>
    )
  }

  return (
    <div className="mb-2 w-full rounded-xl border bg-card/60 shadow-lg backdrop-blur transition-colors hover:bg-card/95 supports-[backdrop-filter]:bg-card/50 supports-[backdrop-filter]:hover:bg-card/85">
      {header}
      {body}
    </div>
  )
}

function WorkflowRunBody({
  run,
  layout,
}: {
  run: WorkflowRun
  layout: "vertical" | "horizontal"
}) {
  const groups = groupAgentsByPhase(run)
  if (groups.length === 0) return null
  if (layout === "horizontal") {
    return (
      <>
        {groups.map((group, i) => (
          <PhaseGroup
            key={group.phase ? group.phase.title : `other-${i}`}
            phase={group.phase}
            agents={group.agents}
            current={isCurrentPhase(run, group.phase)}
            layout="horizontal"
          />
        ))}
      </>
    )
  }
  return (
    <div className="space-y-2">
      {groups.map((group, i) => (
        <PhaseGroup
          key={group.phase ? group.phase.title : `other-${i}`}
          phase={group.phase}
          agents={group.agents}
          current={isCurrentPhase(run, group.phase)}
          layout="vertical"
        />
      ))}
    </div>
  )
}

function isCurrentPhase(
  run: WorkflowRun,
  phase: WorkflowPhase | null
): boolean {
  if (!phase) return false
  return run.current_phase
    ? phase.title === run.current_phase
    : phase.state === "active"
}

function PhaseGroup({
  phase,
  agents,
  current,
  layout,
}: {
  phase: WorkflowPhase | null
  agents: WorkflowAgent[]
  current: boolean
  layout: "vertical" | "horizontal"
}) {
  const t = useTranslations("Folder.chat.workflows")
  const horizontal = layout === "horizontal"
  return (
    <div
      className={cn(
        "rounded-lg border bg-transparent px-2.5 py-2",
        horizontal && "min-w-44 shrink-0"
      )}
    >
      {phase ? (
        <>
          <div className="flex items-start gap-2">
            <PhaseStatusIcon state={phase.state} current={current} />
            <p
              className={cn(
                "min-w-0 flex-1 text-sm leading-5 break-words [overflow-wrap:anywhere]",
                phase.state === "done"
                  ? "text-muted-foreground line-through"
                  : "text-foreground"
              )}
            >
              {phase.title}
            </p>
          </div>
          <div className="mt-2 flex items-center gap-1.5 pl-5">
            <Badge variant="outline" className="h-5 text-3xs uppercase">
              {t(
                phase.state === "done"
                  ? "phaseState.done"
                  : phase.state === "active"
                    ? "phaseState.active"
                    : phase.state === "failed"
                      ? "phaseState.failed"
                      : "phaseState.pending"
              )}
            </Badge>
          </div>
        </>
      ) : null}
      {agents.length > 0 ? (
        <ul
          className={cn(
            horizontal
              ? "mt-2 flex gap-1.5 overflow-x-auto"
              : cn("space-y-1.5", phase ? "mt-2" : undefined)
          )}
        >
          {agents.map((agent) => (
            <li
              key={agent.agent_id}
              className={cn(
                "flex items-start gap-2",
                horizontal && "min-w-28 shrink-0 rounded-md border px-1.5 py-1"
              )}
            >
              <NodeStatusIcon state={agent.state} />
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium leading-4">
                  {agent.label}
                </p>
                <div className="mt-1 flex flex-wrap items-center gap-1">
                  <Badge variant="outline" className="h-5 text-3xs uppercase">
                    {t(
                      agent.state === "done"
                        ? "nodeState.done"
                        : agent.state === "failed"
                          ? "nodeState.failed"
                          : agent.state === "cancelled" ||
                              agent.state === "canceled"
                            ? "nodeState.cancelled"
                            : agent.state === "pending"
                              ? "nodeState.pending"
                              : "nodeState.running"
                    )}
                  </Badge>
                  {typeof agent.tokens_used === "number" &&
                  agent.tokens_used > 0 ? (
                    <span className="text-2xs tabular-nums text-muted-foreground">
                      {formatTokenCount(agent.tokens_used)}
                    </span>
                  ) : null}
                  {typeof agent.duration_ms === "number" &&
                  agent.duration_ms > 0 ? (
                    <NodeDuration ms={agent.duration_ms} />
                  ) : null}
                </div>
              </div>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}

function NodeDuration({ ms }: { ms: number }) {
  const tElapsed = useTranslations("Folder.chat.liveTurnStats")
  return (
    <span className="text-2xs tabular-nums text-muted-foreground">
      {formatElapsedLabel(ms, tElapsed)}
    </span>
  )
}

function PhaseStatusIcon({
  state,
  current,
}: {
  state: string
  current: boolean
}) {
  if (state === "done") {
    return (
      <CheckCircle2Icon className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
    )
  }
  if (state === "failed") {
    return <AlertCircle className="h-3.5 w-3.5 shrink-0 text-destructive" />
  }
  if (state === "active" || current) {
    return (
      <Loader2Icon className="h-3.5 w-3.5 shrink-0 animate-spin text-blue-500" />
    )
  }
  return (
    <CircleDashedIcon className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
  )
}

function NodeStatusIcon({ state }: { state: string }) {
  if (state === "done") {
    return (
      <CheckCircle2Icon className="mt-0.5 h-3 w-3 shrink-0 text-emerald-500" />
    )
  }
  if (state === "failed") {
    return <AlertCircle className="mt-0.5 h-3 w-3 shrink-0 text-destructive" />
  }
  if (state === "cancelled" || state === "canceled" || state === "pending") {
    return (
      <CircleDashedIcon className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground" />
    )
  }
  return (
    <Loader2Icon className="mt-0.5 h-3 w-3 shrink-0 animate-spin text-blue-500" />
  )
}
