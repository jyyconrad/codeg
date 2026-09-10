"use client"

/**
 * Live workflow progress, pinned above the transcript.
 *
 * Distinct from `AsyncTaskStrip`: that strip is the AIR task panel for shells
 * and monitors. This one is the long-running workflow surface — phase rail,
 * agent counts, elapsed time — fed by the canonical `WorkflowRun` table after
 * agent-specific frames have been adapted (Grok `workflow_updated`, Claude
 * `local_workflow`, and any future AIR `taskType=workflow` speaker).
 */

import { useTranslations } from "next-intl"
import { Check, PauseCircle, Workflow } from "lucide-react"

import { Shimmer } from "@/components/ai-elements/shimmer"
import { cn } from "@/lib/utils"
import { formatElapsedLabel } from "@/lib/format-elapsed"
import { liveWorkflows, phaseProgress } from "@/lib/workflow-progress"
import type { WorkflowPhase, WorkflowRun } from "@/lib/types"

export function WorkflowProgressStrip({ runs }: { runs: WorkflowRun[] }) {
  const live = liveWorkflows(runs)
  if (live.length === 0) return null

  return (
    <div className="border-b border-border bg-muted/30">
      {live.map((run) => (
        <WorkflowProgressCard key={run.run_id} run={run} />
      ))}
    </div>
  )
}

function WorkflowProgressCard({ run }: { run: WorkflowRun }) {
  const t = useTranslations("Folder.chat.workflows")
  const tElapsed = useTranslations("Folder.chat.liveTurnStats")
  const paused = run.state === "paused"
  const progress = phaseProgress(run)
  const elapsed =
    typeof run.elapsed_ms === "number" && run.elapsed_ms > 0
      ? formatElapsedLabel(run.elapsed_ms, tElapsed)
      : null
  const agentLine =
    run.agents_done > 0 || run.agents_running > 0
      ? t("agents", { done: run.agents_done, running: run.agents_running })
      : null
  const phaseLine = progress
    ? t("phase", {
        name: run.current_phase ?? t("starting"),
        current: progress.current,
        total: progress.total,
      })
    : (run.last_event ?? run.current_phase ?? t("starting"))

  return (
    <div className="flex flex-col gap-1.5 px-4 py-2 text-xs">
      <div className="flex items-center gap-2">
        {paused ? (
          <PauseCircle
            aria-hidden
            className="size-3.5 shrink-0 text-muted-foreground"
          />
        ) : (
          <Workflow aria-hidden className="size-3.5 shrink-0 text-primary" />
        )}
        <span
          className="min-w-0 truncate font-medium text-foreground"
          title={run.objective || run.name}
        >
          {paused ? (
            run.name
          ) : (
            <Shimmer as="span" duration={1.4} shineColor="var(--primary)">
              {run.name}
            </Shimmer>
          )}
        </span>
        <span className="min-w-0 flex-1 truncate text-muted-foreground/80">
          {[phaseLine, agentLine].filter(Boolean).join(" · ")}
        </span>
        {elapsed ? (
          <span className="shrink-0 tabular-nums text-muted-foreground/70">
            {elapsed}
          </span>
        ) : null}
      </div>
      {run.phases.length > 0 ? (
        <ol className="flex min-w-0 items-center gap-1 overflow-hidden">
          {run.phases.map((phase, i) => (
            <PhaseChip
              key={`${phase.title}-${i}`}
              phase={phase}
              active={
                run.current_phase
                  ? phase.title === run.current_phase
                  : phase.state === "active"
              }
            />
          ))}
        </ol>
      ) : null}
      {progress ? (
        <span
          aria-hidden
          className="h-1 w-full overflow-hidden rounded-full bg-muted"
        >
          <span
            className="block h-full bg-primary transition-[width] duration-300"
            style={{
              width: `${Math.min(100, (progress.current / progress.total) * 100)}%`,
            }}
          />
        </span>
      ) : null}
    </div>
  )
}

function PhaseChip({
  phase,
  active,
}: {
  phase: WorkflowPhase
  active: boolean
}) {
  const done = phase.state === "done"
  const failed = phase.state === "failed"
  return (
    <li
      className={cn(
        "inline-flex min-w-0 shrink items-center gap-1 rounded-full border px-1.5 py-0.5",
        failed
          ? "border-destructive/40 text-destructive"
          : done
            ? "border-border text-muted-foreground"
            : active
              ? "border-primary/40 bg-primary/10 text-foreground"
              : "border-transparent text-muted-foreground/60"
      )}
      title={phase.title}
    >
      {done ? <Check aria-hidden className="size-3 shrink-0" /> : null}
      <span className="truncate">{phase.title}</span>
    </li>
  )
}
