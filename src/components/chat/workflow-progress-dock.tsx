"use client"

/**
 * UI-only dock preference and dismiss state for the live workflow panel.
 *
 * Default is the overlay stack (below the plan card). The user can pin the
 * same panel above the composer. Persistence is local to this browser — it
 * is not a server fact and does not belong on ConnectionState.
 *
 * Dismissing a settled run hides it for this mount only. A later live run
 * with a new id still shows; a late result update on a dismissed id does not.
 */

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react"

import type { WorkflowRun } from "@/lib/types"
import { isWorkflowTerminal } from "@/lib/workflow-progress"

export type WorkflowProgressDock = "overlay" | "composer"

const STORAGE_KEY = "codeg:workflow-progress-dock"

const WorkflowProgressDockContext = createContext<{
  dock: WorkflowProgressDock
  toggleDock: () => void
  runs: WorkflowRun[]
  dismissRuns: (runIds: string[]) => void
} | null>(null)

function readDock(): WorkflowProgressDock {
  if (typeof window === "undefined") return "overlay"
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "composer"
      ? "composer"
      : "overlay"
  } catch {
    return "overlay"
  }
}

export function WorkflowProgressDockProvider({
  runs,
  children,
}: {
  runs: WorkflowRun[]
  children: ReactNode
}) {
  const [dock, setDock] = useState<WorkflowProgressDock>(readDock)
  const [dismissedRunIds, setDismissedRunIds] = useState<ReadonlySet<string>>(
    () => new Set()
  )
  const toggleDock = useCallback(() => {
    setDock((prev) => {
      const next: WorkflowProgressDock =
        prev === "overlay" ? "composer" : "overlay"
      try {
        window.localStorage.setItem(STORAGE_KEY, next)
      } catch {
        /* quota / private mode — preference is still live for this session */
      }
      return next
    })
  }, [])
  const dismissRuns = useCallback((runIds: string[]) => {
    if (runIds.length === 0) return
    setDismissedRunIds((prev) => {
      const next = new Set(prev)
      for (const id of runIds) next.add(id)
      return next
    })
  }, [])
  const visibleRuns = useMemo(
    () =>
      runs.filter(
        (run) => !dismissedRunIds.has(run.run_id) || !isWorkflowTerminal(run)
      ),
    [runs, dismissedRunIds]
  )
  const value = useMemo(
    () => ({ dock, toggleDock, runs: visibleRuns, dismissRuns }),
    [dock, toggleDock, visibleRuns, dismissRuns]
  )
  return (
    <WorkflowProgressDockContext.Provider value={value}>
      {children}
    </WorkflowProgressDockContext.Provider>
  )
}

export function useWorkflowProgressDock() {
  return useContext(WorkflowProgressDockContext)
}
