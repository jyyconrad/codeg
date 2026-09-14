"use client"

/**
 * UI-only dock preference for the live workflow panel.
 *
 * Default is the overlay stack (below the plan card). The user can pin the
 * same panel above the composer. Persistence is local to this browser — it
 * is not a server fact and does not belong on ConnectionState.
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

export type WorkflowProgressDock = "overlay" | "composer"

const STORAGE_KEY = "codeg:workflow-progress-dock"

const WorkflowProgressDockContext = createContext<{
  dock: WorkflowProgressDock
  toggleDock: () => void
  runs: WorkflowRun[]
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
  const value = useMemo(
    () => ({ dock, toggleDock, runs }),
    [dock, toggleDock, runs]
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
