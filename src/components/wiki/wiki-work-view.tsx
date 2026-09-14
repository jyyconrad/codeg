"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { Button } from "@/components/ui/button"
import { toErrorMessage } from "@/lib/app-error"
import {
  subscribeWikiJobChanged,
  wikiListJobs,
  wikiListMemoryNotes,
  wikiListProjectBindings,
} from "@/lib/api"
import { getTransport } from "@/lib/transport"
import {
  groupWikiMemoryNotesByProject,
  latestFailedWikiSynthesizeJob,
  normalizeWikiList,
  wikiJobErrorMessage,
  wikiMemoryNoteKey,
  wikiMemoryNoteTitle,
  wikiMemoryPageTypeKey,
  wikiProjectDisplayTitle,
  type WikiJob,
  type WikiMemoryNote,
  type WikiProjectBinding,
} from "@/lib/wiki-types"
import { WikiNoteBrowser } from "./wiki-shared"

function asProjectBindings(raw: unknown): WikiProjectBinding[] {
  if (Array.isArray(raw)) return raw as WikiProjectBinding[]
  return normalizeWikiList<WikiProjectBinding>(raw).items
}

export function WikiWorkView() {
  const t = useTranslations("Wiki")
  const [projects, setProjects] = useState<WikiProjectBinding[]>([])
  const [notes, setNotes] = useState<WikiMemoryNote[]>([])
  const [synthesizeError, setSynthesizeError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const load = useCallback(
    async (opts?: { silent?: boolean }) => {
      if (!opts?.silent) {
        setLoading(true)
        setError(null)
      }
      try {
        const [projectRaw, notesRaw, jobsRaw] = await Promise.all([
          wikiListProjectBindings().catch(() => []),
          wikiListMemoryNotes(),
          wikiListJobs({ status: "failed", limit: 20 }).catch(() => []),
        ])
        setProjects(asProjectBindings(projectRaw))
        setNotes(normalizeWikiList<WikiMemoryNote>(notesRaw).items)
        const failed = latestFailedWikiSynthesizeJob(
          normalizeWikiList<WikiJob>(jobsRaw).items
        )
        setSynthesizeError(wikiJobErrorMessage(failed))
        setError(null)
      } catch (err) {
        if (!opts?.silent) {
          setNotes([])
          setError(t("loadFailed", { message: toErrorMessage(err) }))
        }
      } finally {
        if (!opts?.silent) setLoading(false)
      }
    },
    [t]
  )

  useEffect(() => {
    load().catch(console.error)
  }, [load])

  useEffect(() => {
    let cancelled = false
    let unsub: (() => void) | undefined

    void subscribeWikiJobChanged(() => {
      if (!cancelled) load({ silent: true }).catch(console.error)
    }).then((dispose) => {
      if (cancelled) {
        dispose()
        return
      }
      unsub = dispose
    })

    const offReconnect = getTransport().onReconnect?.(() => {
      if (!cancelled) load({ silent: true }).catch(console.error)
    })

    return () => {
      cancelled = true
      unsub?.()
      offReconnect?.()
    }
  }, [load])

  const grouped = useMemo(
    () => groupWikiMemoryNotesByProject(notes, projects),
    [notes, projects]
  )
  const hasMemory = grouped.groups.length > 0 || grouped.ungrouped.length > 0

  return (
    <div className="flex h-full min-h-0 flex-col">
      {synthesizeError ? (
        <div className="shrink-0 border-b bg-destructive/10 px-4 py-2 text-sm text-destructive">
          {t("work.compileFailed", { message: synthesizeError })}
        </div>
      ) : null}
      {loading ? (
        <div className="flex shrink-0 items-center gap-2 border-b px-4 py-3 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin" />
          {t("loading")}
        </div>
      ) : error ? (
        <div className="flex shrink-0 items-center gap-2 border-b px-4 py-3 text-sm text-destructive">
          <p className="min-w-0 flex-1">{error}</p>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => load().catch(console.error)}
          >
            {t("retry")}
          </Button>
        </div>
      ) : hasMemory ? (
        <div className="max-h-[45%] shrink-0 space-y-3 overflow-y-auto border-b p-4">
          <div className="grid gap-3 md:grid-cols-2">
            {grouped.groups.map(({ project, notes: projectNotes }) => (
              <MemoryProjectCard
                key={project.id}
                title={wikiProjectDisplayTitle(project)}
                path={project.root_folder_path}
                notes={projectNotes}
              />
            ))}
            {grouped.ungrouped.length > 0 ? (
              <MemoryProjectCard
                key="ungrouped"
                title={t("work.ungrouped")}
                notes={grouped.ungrouped}
              />
            ) : null}
          </div>
        </div>
      ) : null}
      <div className="min-h-0 flex-1">
        <WikiNoteBrowser
          prefix="work"
          emptyTitle={t("work.emptyTitle")}
          emptyDescription={t("work.emptyDescription")}
        />
      </div>
    </div>
  )
}

function MemoryProjectCard({
  title,
  path,
  notes,
}: {
  title: string
  path?: string | null
  notes: WikiMemoryNote[]
}) {
  const t = useTranslations("Wiki")
  return (
    <article className="rounded-lg border p-4">
      <h3 className="font-medium">{title}</h3>
      {path ? (
        <p className="mt-1 truncate text-xs text-muted-foreground">{path}</p>
      ) : null}
      <p className="mt-3 text-sm">
        {t("work.memoryCount", { count: notes.length })}
      </p>
      <ul className="mt-2 space-y-2">
        {notes.map((note) => {
          const pageType = wikiMemoryPageTypeKey(note.page_type)
          const summary = note.summary?.trim()
          return (
            <li key={wikiMemoryNoteKey(note)} className="text-sm">
              <div className="flex items-baseline gap-2">
                <span className="min-w-0 flex-1 font-medium">
                  {wikiMemoryNoteTitle(note)}
                </span>
                {pageType !== "unknown" ? (
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {t(`work.pageType.${pageType}`)}
                  </span>
                ) : null}
              </div>
              {summary ? (
                <p className="mt-0.5 line-clamp-3 text-xs text-muted-foreground">
                  {summary}
                </p>
              ) : null}
            </li>
          )
        })}
      </ul>
    </article>
  )
}
