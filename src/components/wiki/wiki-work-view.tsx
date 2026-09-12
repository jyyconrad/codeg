"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"

import { wikiListProjectBindings, wikiListSources } from "@/lib/api"
import {
  normalizeWikiList,
  type WikiProjectBinding,
  type WikiSource,
} from "@/lib/wiki-types"

export function WikiWorkView() {
  const t = useTranslations("Wiki")
  const [projects, setProjects] = useState<WikiProjectBinding[]>([])
  const [sources, setSources] = useState<WikiSource[]>([])
  useEffect(() => {
    wikiListProjectBindings()
      .then(setProjects)
      .catch(() => setProjects([]))
    wikiListSources({ source_kind: "acp-turn" })
      .then((raw) => setSources(normalizeWikiList<WikiSource>(raw).items))
      .catch(() => setSources([]))
  }, [])
  if (projects.length === 0) {
    return (
      <div className="p-6 text-sm text-muted-foreground">
        {t("work.emptyTitle")}
      </div>
    )
  }
  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4">
      <p className="text-sm text-muted-foreground">
        {t("work.emptyDescription")}
      </p>
      <div className="grid gap-3 md:grid-cols-2">
        {projects.map((project) => {
          const linked = sources.filter((s) =>
            s.project_ids?.includes(project.id)
          )
          return (
            <article key={project.id} className="rounded-lg border p-4">
              <h3 className="font-medium">
                {project.project_note_id || `Project ${project.root_folder_id}`}
              </h3>
              <p className="mt-1 text-xs text-muted-foreground">
                root folder #{project.root_folder_id}
              </p>
              <p className="mt-3 text-sm">
                {linked.length} source{linked.length === 1 ? "" : "s"}
              </p>
              {linked.length > 0 && (
                <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
                  {linked.slice(0, 5).map((source) => (
                    <li key={source.id}>
                      {source.title || source.source_title || source.id}
                    </li>
                  ))}
                </ul>
              )}
            </article>
          )
        })}
      </div>
    </div>
  )
}
