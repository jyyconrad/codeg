/**
 * 工作、能力及搜索结果共用的笔记列表，按类型或项目筛选并按页加载。
 * 后端提供笔记摘要与项目绑定；选中项由共享 URL 路由驱动，正文交给 WikiNoteReader。
 */
"use client"

import { useEffect, useRef, useState } from "react"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type {
  WikiListPage,
  WikiNoteSummary,
  WikiProjectBinding,
} from "@/lib/wiki-types"
import { wikiProjectDisplayTitle } from "@/lib/wiki-types"
import { useWikiData, useWikiQuery } from "./wiki-data"
import { WikiNoteCard, WikiNoteReader } from "./wiki-note-reader"
import { WikiCompileButton } from "./wiki-jobs-view"
import { WikiEmptyState } from "./wiki-shared"

const NO_NOTES: WikiNoteSummary[] = []

export function WikiNotesView({
  view,
}: {
  view: "overview" | "work" | "capabilities"
}) {
  const t = useTranslations("Wiki.v2")
  const { route, navigate, overview } = useWikiData()
  const [type, setType] = useState("")
  const [project, setProject] = useState("")
  const [limit, setLimit] = useState(50)
  const autoSelected = useRef(false)
  const { data, error, loading, refresh } = useWikiQuery<
    WikiListPage<WikiNoteSummary>
  >("wiki_list_notes", {
    query: {
      view,
      type: type || undefined,
      project_id: project || undefined,
      query: route.query,
      scope: route.query ? route.scope : "notes",
      limit,
      offset: 0,
    },
  })
  const projects = useWikiQuery<WikiProjectBinding[]>(
    "wiki_list_project_bindings",
    {},
    view === "work"
  )
  const items = data?.items ?? NO_NOTES
  const select = (note: WikiNoteSummary, replace = false) =>
    navigate(
      note.type === "source" && note.source_id
        ? { view: "sources", source: note.source_id, path: null }
        : { path: note.path, source: null },
      replace
    )
  useEffect(() => {
    if (
      !autoSelected.current &&
      !route.path &&
      !route.query &&
      items.length &&
      window.matchMedia("(min-width: 768px)").matches
    ) {
      autoSelected.current = true
      const note = items[0]
      navigate({ path: note.path, source: null }, true)
    }
  }, [items, route.path, route.query, navigate])
  return (
    <div className="flex h-full min-h-0 min-w-0">
      <aside
        className={`min-h-0 min-w-0 flex-col overflow-y-auto border-e md:w-80 md:shrink-0 ${route.path ? "hidden md:flex" : "flex w-full"}`}
      >
        <div className="space-y-3 border-b p-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold">
              {route.query
                ? t("searchResults", { count: data?.total ?? 0 })
                : t(`nav.${view}`)}
            </h2>
            <Button size="sm" variant="ghost" onClick={refresh}>
              {t("refresh")}
            </Button>
          </div>
          {view !== "capabilities" && (
            <Select
              value={type || "all"}
              onValueChange={(value) => {
                setType(value === "all" ? "" : value)
                setLimit(50)
                navigate({ path: null }, true)
              }}
            >
              <SelectTrigger aria-label={t("noteType")} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                <SelectItem value="all">{t("allTypes")}</SelectItem>
                {(
                  [
                    "turn-summary",
                    "session-summary",
                    "project",
                    "work-record",
                    "method",
                    "concept",
                  ] as const
                ).map((value) => (
                  <SelectItem key={value} value={value}>
                    {t(`types.${value}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          {view === "work" && (projects.data?.length ?? 0) > 0 && (
            <Select
              value={project || "all"}
              onValueChange={(value) => {
                setProject(value === "all" ? "" : value)
                setLimit(50)
                navigate({ path: null }, true)
              }}
            >
              <SelectTrigger aria-label={t("project")} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="start">
                <SelectItem value="all">{t("allProjects")}</SelectItem>
                {projects.data?.map((item) => (
                  <SelectItem key={item.id} value={item.id}>
                    {wikiProjectDisplayTitle(item)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>
        {error && (
          <div role="alert" className="p-4 text-sm text-destructive">
            {error}
            <Button size="sm" variant="outline" onClick={refresh}>
              {t("retry")}
            </Button>
          </div>
        )}
        {loading && !data && (
          <p role="status" className="p-4 text-sm">
            {t("loading")}
          </p>
        )}
        {!loading && !error && !items.length && (
          <WikiEmptyState
            title={t(
              route.query
                ? "noResults"
                : view === "capabilities"
                  ? "noCapabilities"
                  : "noNotes"
            )}
            description={t(
              route.query
                ? "searchHint"
                : view === "capabilities"
                  ? "noCapabilitiesHint"
                  : "noNotesHint"
            )}
          >
            {view === "capabilities" && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => navigate({ view: "work", path: null })}
              >
                {t("viewWork")}
              </Button>
            )}
            {view === "capabilities" && !!overview?.pending_memory_count && (
              <WikiCompileButton />
            )}
          </WikiEmptyState>
        )}
        <ul className="space-y-1 p-2">
          {items.map((note) => (
            <li key={`${note.note_id}:${note.path}`}>
              <WikiNoteCard
                note={note}
                projectNames={(projects.data ?? [])
                  .filter((project) => note.project_ids.includes(project.id))
                  .map(wikiProjectDisplayTitle)}
                selected={route.path === note.path}
                onClick={() => select(note)}
              />
            </li>
          ))}
        </ul>
        {items.length < (data?.total ?? 0) && (
          <Button
            className="m-3"
            variant="outline"
            disabled={loading}
            onClick={() => setLimit((value) => value + 50)}
          >
            {t("loadMore")}
          </Button>
        )}
      </aside>
      <div
        className={`min-h-0 min-w-0 flex-1 overflow-y-auto ${!route.path ? "hidden md:block" : ""}`}
      >
        {route.path ? (
          <WikiNoteReader key={route.path} path={route.path} />
        ) : (
          <WikiEmptyState
            title={t("selectNote")}
            description={t("selectNoteHint")}
          />
        )}
      </div>
    </div>
  )
}
