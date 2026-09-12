"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"

import { wikiListSources } from "@/lib/api"
import { normalizeWikiList, type WikiSource } from "@/lib/wiki-types"

export function WikiCapabilitiesView() {
  const t = useTranslations("Wiki")
  const [sources, setSources] = useState<WikiSource[]>([])
  useEffect(() => {
    wikiListSources({ source_kind: "acp-turn" })
      .then((raw) => setSources(normalizeWikiList<WikiSource>(raw).items))
      .catch(() => setSources([]))
  }, [])
  const practiced = sources.filter(
    (s) => Boolean(s.personal_role) && s.material_role === "own-work"
  )
  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-4">
      <p className="text-sm text-muted-foreground">
        {t("capabilities.emptyDescription")}
      </p>
      <div className="rounded-lg border">
        <div className="border-b px-4 py-3 text-sm font-medium">
          {t("views.capabilities")}
        </div>
        {practiced.length === 0 ? (
          <div className="p-4 text-sm text-muted-foreground">
            {t("capabilities.emptyTitle")}
          </div>
        ) : (
          <ul className="divide-y">
            {practiced.map((source) => (
              <li key={source.id} className="px-4 py-3">
                <div className="text-sm font-medium">
                  {source.personal_role}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {source.title || source.source_title || source.id}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {t("capabilities.evidenceApplication")}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
