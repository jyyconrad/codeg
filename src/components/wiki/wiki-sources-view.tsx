"use client"

import { useCallback, useEffect, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Textarea } from "@/components/ui/textarea"
import { toErrorMessage } from "@/lib/app-error"
import { wikiGetSource, wikiListSources, wikiVaultRead } from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  normalizeWikiList,
  wikiSourcePreviewPaths,
  wikiSourceTitle,
  wikiVaultReadContent,
  type WikiSource,
} from "@/lib/wiki-types"
import { WikiEmptyState, WikiMarkdownPreview } from "./wiki-shared"

const PAGE_SIZE = 50

function sourceKindKey(
  kind: string | null | undefined
): "acp-turn" | "document" | "pasted-text" | "unknown" {
  if (kind === "acp-turn" || kind === "document" || kind === "pasted-text") {
    return kind
  }
  return "unknown"
}

function eligibilityKey(
  value: string | null | undefined
):
  | "processing"
  | "awaiting-acceptance"
  | "ready"
  | "failed"
  | "cancelled"
  | "withdrawn"
  | "unknown" {
  switch (value) {
    case "processing":
    case "awaiting-acceptance":
    case "ready":
    case "failed":
    case "cancelled":
    case "withdrawn":
      return value
    default:
      return "unknown"
  }
}

export function WikiSourcesView() {
  const t = useTranslations("Wiki")
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [sources, setSources] = useState<WikiSource[]>([])
  const [total, setTotal] = useState(0)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [preview, setPreview] = useState("")
  const [previewPath, setPreviewPath] = useState<string | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)

  const load = useCallback(
    async (offset: number) => {
      const appending = offset > 0
      if (appending) setLoadingMore(true)
      else {
        setLoading(true)
        setError(null)
      }
      try {
        const page = normalizeWikiList<WikiSource>(
          await wikiListSources({ limit: PAGE_SIZE, offset })
        )
        setTotal(page.total)
        setSources((current) =>
          appending ? [...current, ...page.items] : page.items
        )
      } catch (err) {
        if (!appending) {
          setSources([])
          setTotal(0)
        }
        setError(t("loadFailed", { message: toErrorMessage(err) }))
      } finally {
        setLoading(false)
        setLoadingMore(false)
      }
    },
    [t]
  )

  useEffect(() => {
    load(0).catch(console.error)
  }, [load])

  const selectSource = useCallback(
    async (source: WikiSource) => {
      setSelectedId(source.id)
      setPreviewLoading(true)
      setPreviewError(null)
      setPreview("")
      setPreviewPath(null)
      try {
        const detail = await wikiGetSource(source.id).catch(() => source)
        const paths = wikiSourcePreviewPaths(detail)
        let lastError: unknown = null
        let foundPath: string | null = null
        for (const path of paths) {
          try {
            const content = wikiVaultReadContent(await wikiVaultRead(path))
            setPreview(content)
            setPreviewPath(path)
            foundPath = path
            lastError = null
            break
          } catch (err) {
            lastError = err
          }
        }
        if (lastError && !foundPath) {
          setPreviewError(
            t("loadFailed", { message: toErrorMessage(lastError) })
          )
        }
      } catch (err) {
        setPreviewError(t("loadFailed", { message: toErrorMessage(err) }))
      } finally {
        setPreviewLoading(false)
      }
    },
    [t]
  )

  const selected = sources.find((row) => row.id === selectedId) ?? null

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WikiEmptyState
        title={t("sources.emptyTitle")}
        description={t("sources.emptyDescription")}
      >
        <div className="space-y-2">
          <Button type="button" size="sm" disabled>
            {t("sources.importComingSoon")}
          </Button>
          <Textarea
            disabled
            placeholder={t("sources.pastePlaceholder")}
            className="min-h-20"
          />
          <p className="text-xs leading-5 text-muted-foreground">
            {t("sources.importHint")}
          </p>
        </div>
      </WikiEmptyState>
      <div className="flex min-h-0 flex-1 flex-col border-t md:flex-row">
        <div className="flex min-h-0 w-full flex-col border-b md:w-80 md:border-b-0 md:border-r">
          <div className="flex items-center justify-between gap-2 px-3 py-2">
            <p className="text-xs font-medium text-muted-foreground">
              {t("sources.listTitle", { count: total || sources.length })}
            </p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => load(0).catch(console.error)}
            >
              {t("refresh")}
            </Button>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            {loading ? (
              <div className="flex items-center gap-2 px-3 py-4 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : error ? (
              <div className="space-y-2 px-3 py-4 text-sm text-destructive">
                <p>{error}</p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => load(0).catch(console.error)}
                >
                  {t("retry")}
                </Button>
              </div>
            ) : sources.length === 0 ? (
              <p className="px-3 py-4 text-sm text-muted-foreground">
                {t("empty")}
              </p>
            ) : (
              <ul className="space-y-1 px-2 pb-3">
                {sources.map((source) => {
                  const kind = sourceKindKey(source.source_kind)
                  const eligibility = eligibilityKey(source.eligibility)
                  return (
                    <li key={source.id}>
                      <button
                        type="button"
                        className={cn(
                          "flex w-full flex-col gap-1 rounded-lg px-2 py-2 text-left hover:bg-muted/60",
                          selectedId === source.id && "bg-muted"
                        )}
                        onClick={() => {
                          selectSource(source).catch(console.error)
                        }}
                      >
                        <span className="truncate text-sm font-medium">
                          {wikiSourceTitle(source) ?? t("sources.untitled")}
                        </span>
                        <span className="flex flex-wrap gap-1">
                          <Badge variant="outline">
                            {t(`sources.kind.${kind}`)}
                          </Badge>
                          {source.eligibility ? (
                            <Badge
                              variant={
                                eligibility === "failed"
                                  ? "destructive"
                                  : "secondary"
                              }
                            >
                              {t(`sources.eligibility.${eligibility}`)}
                            </Badge>
                          ) : null}
                        </span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
            {sources.length < total ? (
              <div className="px-3 pb-3">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={loadingMore}
                  onClick={() => load(sources.length).catch(console.error)}
                >
                  {loadingMore ? t("loading") : t("loadMore")}
                </Button>
              </div>
            ) : null}
          </ScrollArea>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="p-4">
            <p className="mb-3 text-xs font-medium text-muted-foreground">
              {t("sources.previewTitle")}
              {previewPath ? ` · ${previewPath}` : ""}
            </p>
            {!selected ? (
              <p className="text-sm text-muted-foreground">
                {t("note.preview")}
              </p>
            ) : previewLoading ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : previewError ? (
              <p className="text-sm text-destructive">{previewError}</p>
            ) : (
              <WikiMarkdownPreview content={preview} />
            )}
          </div>
        </ScrollArea>
      </div>
    </div>
  )
}
