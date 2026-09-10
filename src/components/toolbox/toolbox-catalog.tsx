"use client"

import { Star } from "lucide-react"
import { useTranslations } from "next-intl"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { cn } from "@/lib/utils"
import { listToolboxTools } from "./registry"
import { matchesToolboxQuery } from "./search"
import { TOOLBOX_CATEGORIES, type ToolboxToolId } from "./types"
import { useToolboxStore } from "./toolbox-store"

export function ToolboxCatalog({
  query,
  onQueryChange,
}: {
  query: string
  onQueryChange: (value: string) => void
}) {
  const t = useTranslations("Toolbox")
  const selectedToolId = useToolboxStore((s) => s.selectedToolId)
  const selectTool = useToolboxStore((s) => s.selectTool)
  const favorites = useToolboxStore((s) => s.favorites)
  const recent = useToolboxStore((s) => s.recent)
  const toggleFavorite = useToolboxStore((s) => s.toggleFavorite)

  const visible = listToolboxTools().filter((tool) =>
    matchesToolboxQuery(
      tool,
      {
        title: t(`tools.${tool.id}.title`),
        description: t(`tools.${tool.id}.description`),
        aliases: t(`tools.${tool.id}.aliases`),
        category: t(`categories.${tool.category}`),
      },
      query
    )
  )
  const visibleIds = new Set(visible.map((tool) => tool.id))

  function row(id: ToolboxToolId) {
    const active = selectedToolId === id
    const fav = favorites.includes(id)
    return (
      <div key={id} className="flex items-center gap-0.5 pr-1">
        <button
          type="button"
          onClick={() => selectTool(id)}
          className={cn(
            "flex min-w-0 flex-1 items-center rounded-full px-2 py-1.5 text-left text-[0.8125rem]",
            "hover:bg-sidebar-accent",
            active && "bg-sidebar-primary/8"
          )}
        >
          <span className="truncate">{t(`tools.${id}.title`)}</span>
        </button>
        <button
          type="button"
          aria-label={t("favorite")}
          aria-pressed={fav}
          onClick={() => toggleFavorite(id)}
          className="rounded-full p-1 text-muted-foreground hover:text-foreground"
        >
          <Star
            className={cn("size-3.5", fav && "fill-amber-400 text-amber-400")}
          />
        </button>
      </div>
    )
  }

  const favoriteRows = favorites.filter((id) => visibleIds.has(id))
  const recentRows = recent.filter(
    (id) => visibleIds.has(id) && !favorites.includes(id)
  )

  return (
    <div className="flex h-full min-h-0 w-[16.5rem] shrink-0 flex-col border-r border-border/60">
      <div className="p-2">
        <Input
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder={t("searchPlaceholder")}
          aria-label={t("searchPlaceholder")}
        />
      </div>
      <ScrollArea className="min-h-0 flex-1" y="scroll">
        <div className="flex flex-col gap-3 px-2 pb-3">
          {favoriteRows.length > 0 ? (
            <section>
              <h2 className="px-2 pb-1 text-[0.6875rem] font-medium uppercase tracking-wide text-muted-foreground">
                {t("favorites")}
              </h2>
              {favoriteRows.map(row)}
            </section>
          ) : null}
          {recentRows.length > 0 ? (
            <section>
              <h2 className="px-2 pb-1 text-[0.6875rem] font-medium uppercase tracking-wide text-muted-foreground">
                {t("recent")}
              </h2>
              {recentRows.map(row)}
            </section>
          ) : null}
          {TOOLBOX_CATEGORIES.map((category) => {
            const tools = visible.filter((tool) => tool.category === category)
            if (tools.length === 0) return null
            return (
              <section key={category}>
                <h2 className="px-2 pb-1 text-[0.6875rem] font-medium uppercase tracking-wide text-muted-foreground">
                  {t(`categories.${category}`)}
                </h2>
                {tools.map((tool) => row(tool.id))}
              </section>
            )
          })}
          {visible.length === 0 ? (
            <p className="px-2 text-sm text-muted-foreground">
              {t("noResults")}
            </p>
          ) : null}
        </div>
      </ScrollArea>
    </div>
  )
}
