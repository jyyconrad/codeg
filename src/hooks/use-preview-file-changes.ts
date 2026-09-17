"use client"

import { useEffect, useRef } from "react"

import { useWorkspaceStateStore } from "@/hooks/use-workspace-state-store"
import {
  findOwningFolder,
  joinRootRel,
  normalizeAbsPath,
} from "@/lib/file-open-target"
import { useAppWorkspaceStore } from "@/stores/app-workspace-store"

const DEBOUNCE_MS = 400

/** Reload a binary/spreadsheet preview when the workspace watcher reports
 *  the open file changed. No-op when the file is outside every registered
 *  folder (no stream to subscribe to). */
export function usePreviewFileChanges(
  absPath: string | null,
  onChange: () => void
): void {
  const allFolders = useAppWorkspaceStore((s) => s.allFolders)
  const owning = absPath ? findOwningFolder(absPath, allFolders) : null
  const root = owning?.rootPath ?? null
  const store = useWorkspaceStateStore(root, "paths")
  const subscribeEnvelopes = store.subscribeEnvelopes
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  useEffect(() => {
    if (!absPath || !root) return
    let timer: ReturnType<typeof setTimeout> | null = null
    const unsub = subscribeEnvelopes(({ changed_paths }) => {
      if (!changed_paths?.length) return
      const hit = changed_paths.some(
        (changed) => joinRootRel(root, changed) === normalizeAbsPath(absPath)
      )
      if (!hit) return
      if (timer != null) clearTimeout(timer)
      timer = setTimeout(() => onChangeRef.current(), DEBOUNCE_MS)
    })
    return () => {
      unsub()
      if (timer != null) clearTimeout(timer)
    }
  }, [absPath, root, subscribeEnvelopes])
}
