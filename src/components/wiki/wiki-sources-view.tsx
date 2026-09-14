/**
 * Wiki 素材阅读与处理视图，展示提取正文、原文、来源信息和关联笔记。
 * 后端登记素材身份与状态；本页通过专用 API 接受部分提取或重新提取，并保留失败提示。
 */
"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import {
  wikiAcceptExtraction,
  wikiReadNote,
  wikiReextract,
} from "@/lib/wiki-api"
import { wikiBodyForReading, wikiSourceLineExcerpt } from "@/lib/wiki-content"
import { toErrorMessage } from "@/lib/app-error"
import {
  wikiSourceTitle,
  type WikiSource,
  type WikiListPage,
  type WikiSourceDocument,
} from "@/lib/wiki-types"
import { useWikiData, useWikiQuery } from "./wiki-data"
import { WikiDate, WikiNoteCard, WikiNoteReader } from "./wiki-note-reader"
import { WikiEmptyState, WikiMarkdownPreview } from "./wiki-shared"

function isSessionMaterial(kind?: string | null): boolean {
  return kind === "acp-turn" || kind === "local-session"
}

export function WikiSourcesView() {
  const t = useTranslations("Wiki.v2")
  const old = useTranslations("Wiki.sources")
  const { route, navigate, invalidate } = useWikiData()
  const [limit, setLimit] = useState(50)
  const [tab, setTab] = useState("body")
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const list = useWikiQuery<WikiListPage<WikiSource>>(
    "wiki_list_sources_page",
    { query: route.query, offset: 0, limit }
  )
  const detail = useWikiQuery<WikiSourceDocument>(
    "wiki_read_source_document",
    { sourceId: route.source },
    !!route.source
  )
  useEffect(() => {
    setTab("body")
    setMessage(null)
    setActionError(null)
  }, [route.source])
  const [anchor, setAnchor] = useState(() =>
    typeof window === "undefined" ? "" : window.location.hash
  )
  useEffect(() => {
    const changed = () => setAnchor(window.location.hash)
    window.addEventListener("hashchange", changed)
    return () => window.removeEventListener("hashchange", changed)
  }, [])
  const excerpt = detail.data
    ? wikiSourceLineExcerpt(detail.data.raw, anchor)
    : null
  const openNoteLink = async (path: string, hash: string) => {
    try {
      await wikiReadNote(path)
      navigate({ path })
      if (hash) window.location.hash = hash
    } catch {
      setActionError(t("missingLink"))
    }
  }
  const source = detail.data?.source
  const readError = detail.data?.read_error
  const activeTab = readError && tab === "body" ? "info" : tab
  const mutate = async (action: "accept" | "extract") => {
    if (!source) return
    setBusy(true)
    setActionError(null)
    try {
      await (action === "accept"
        ? wikiAcceptExtraction(source.id)
        : wikiReextract(source.id))
      setMessage(t(action === "accept" ? "accepted" : "extracted"))
      invalidate()
    } catch (err) {
      setActionError(toErrorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  if (route.path)
    return (
      <div className="h-full overflow-y-auto">
        <WikiNoteReader key={route.path} path={route.path} />
      </div>
    )
  return (
    <div className="flex h-full min-h-0 min-w-0">
      <aside
        className={`min-h-0 overflow-y-auto border-e md:w-80 md:shrink-0 ${route.source ? "hidden md:block" : "w-full"}`}
      >
        <div className="flex items-center justify-between border-b p-3">
          <h2 className="text-sm font-semibold">
            {t("nav.sources")} · {list.data?.total ?? 0}
          </h2>
          <Button size="sm" variant="ghost" onClick={invalidate}>
            {t("refresh")}
          </Button>
        </div>
        {list.error && (
          <p role="alert" className="p-4 text-sm text-destructive">
            {list.error}
          </p>
        )}
        {list.loading && !list.data && (
          <p role="status" className="p-4">
            {t("loading")}
          </p>
        )}
        {list.data && !list.data.items.length && (
          <WikiEmptyState
            title={t("noSources")}
            description={t("archiveHint")}
          />
        )}
        <ul className="space-y-2 p-2">
          {list.data?.items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                aria-current={route.source === item.id ? "page" : undefined}
                className={`w-full space-y-2 rounded-lg p-3 text-start hover:bg-muted/60 ${route.source === item.id ? "bg-muted" : ""}`}
                onClick={() => navigate({ source: item.id, path: null })}
              >
                <span className="block break-words text-sm font-semibold">
                  {wikiSourceTitle(item) || t("untitled")}
                </span>
                <span className="flex flex-wrap gap-1">
                  <Badge variant="outline">
                    {isSessionMaterial(item.source_kind)
                      ? t("sessionMaterial")
                      : t("archiveOnly")}
                  </Badge>
                  {item.extraction_status === "partial" && (
                    <Badge variant="secondary">{t("partial")}</Badge>
                  )}
                  {item.eligibility === "failed" && (
                    <Badge variant="destructive">{t("needsAttention")}</Badge>
                  )}
                </span>
                <span className="block text-xs text-muted-foreground">
                  <WikiDate value={item.captured_at} />
                </span>
              </button>
            </li>
          ))}
        </ul>
        {(list.data?.items.length ?? 0) < (list.data?.total ?? 0) && (
          <Button
            className="m-3"
            variant="outline"
            disabled={list.loading}
            onClick={() => setLimit((value) => value + 50)}
          >
            {t("loadMore")}
          </Button>
        )}
      </aside>
      <div
        className={`min-h-0 min-w-0 flex-1 overflow-y-auto ${route.source ? "" : "hidden md:block"}`}
      >
        <article className="mx-auto max-w-[840px] p-4 pb-12 md:p-6">
          {!route.source && (
            <WikiEmptyState
              title={t("selectSource")}
              description={t("archiveHint")}
            />
          )}
          {route.source && (
            <Button
              size="sm"
              variant="ghost"
              className="mb-4"
              onClick={() => navigate({ source: null })}
            >
              <ArrowLeft className="size-4 rtl:rotate-180" />
              {t("backToList")}
            </Button>
          )}
          {detail.error && (
            <div role="alert" className="space-y-2 text-sm text-destructive">
              <p>
                {t("readFailed")}: {detail.error}
              </p>
              <Button variant="outline" size="sm" onClick={invalidate}>
                {t("retry")}
              </Button>
            </div>
          )}
          {detail.loading && !detail.data && (
            <p role="status">{t("loading")}</p>
          )}
          {source && detail.data && (
            <>
              <h1 className="text-2xl font-semibold break-words">
                {wikiSourceTitle(source) || t("untitled")}
              </h1>
              <p className="my-3 text-xs text-muted-foreground">
                <WikiDate value={source.captured_at} />
              </p>
              <div className="my-4 rounded-lg border bg-muted/30 p-3 text-sm">
                <span className="font-medium">
                  {isSessionMaterial(source.source_kind)
                    ? t("sessionMaterial")
                    : t("archiveOnly")}
                </span>
                <p className="mt-1 text-muted-foreground">
                  {isSessionMaterial(source.source_kind)
                    ? t("sessionMaterialHint")
                    : t("archiveHint")}
                </p>
              </div>
              {readError && (
                <div
                  role="alert"
                  className="my-4 space-y-2 rounded-lg border border-destructive/30 p-3 text-sm"
                >
                  <p>{t("readFailed")}</p>
                  <p className="text-muted-foreground">
                    {source.eligibility === "failed"
                      ? t("noExtractedText")
                      : readError}
                  </p>
                  {!isSessionMaterial(source.source_kind) && (
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy}
                      onClick={() => void mutate("extract")}
                    >
                      {t("extractAgain")}
                    </Button>
                  )}
                </div>
              )}
              {source.extraction_status === "partial" && (
                <div className="my-4 space-y-3 rounded-lg border border-amber-500/40 p-3 text-sm">
                  <p>{t("partialHint")}</p>
                  <div className="flex flex-wrap gap-2">
                    {source.eligibility === "awaiting-acceptance" && (
                      <Button
                        size="sm"
                        disabled={busy}
                        onClick={() => void mutate("accept")}
                      >
                        {t("accept")}
                      </Button>
                    )}
                    <Button
                      size="sm"
                      variant="outline"
                      disabled={busy}
                      onClick={() => void mutate("extract")}
                    >
                      {t("extractAgain")}
                    </Button>
                  </div>
                </div>
              )}
              {message && (
                <p role="status" className="my-3 text-sm">
                  {message}
                </p>
              )}
              {actionError && (
                <p role="alert" className="my-3 text-sm text-destructive">
                  {actionError}
                </p>
              )}
              <div
                className="my-5 flex flex-wrap gap-2"
                role="group"
                aria-label={t("sourceSections")}
              >
                {(["body", "info", "related"] as const).map((value) => (
                  <Button
                    key={value}
                    size="sm"
                    variant={activeTab === value ? "secondary" : "ghost"}
                    aria-pressed={activeTab === value}
                    disabled={value === "body" && !!readError}
                    onClick={() => setTab(value)}
                  >
                    {t(`sourceTabs.${value}`)}
                  </Button>
                ))}
              </div>
              {activeTab === "body" && (
                <>
                  {detail.data.format_warning && (
                    <p className="mb-4 text-sm">{t("formatWarning")}</p>
                  )}
                  {excerpt && (
                    <section className="my-4 rounded border bg-muted/30 p-3">
                      <p className="mb-2 text-sm font-medium">
                        {t("lines", { start: excerpt.start, end: excerpt.end })}
                      </p>
                      <pre className="whitespace-pre-wrap break-words text-sm">
                        {excerpt.text}
                      </pre>
                    </section>
                  )}
                  {detail.data.body.trim() ? (
                    <WikiMarkdownPreview
                      content={wikiBodyForReading(
                        detail.data.body,
                        wikiSourceTitle(source) || ""
                      )}
                      onOpenWikilink={(path, hash) =>
                        void openNoteLink(path, hash)
                      }
                      path={source.raw_path ?? ""}
                    />
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      {t("noExtractedText")}
                    </p>
                  )}
                  <details className="mt-6 text-sm">
                    <summary className="cursor-pointer text-muted-foreground">
                      {t("sourceCode")}
                    </summary>
                    <pre className="mt-3 max-w-full overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-4 text-xs">
                      {detail.data.raw}
                    </pre>
                  </details>
                </>
              )}
              {activeTab === "info" && (
                <div className="space-y-4">
                  <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-3 text-sm">
                    {[
                      [
                        t("sourceName"),
                        source.original_filename || wikiSourceTitle(source),
                      ],
                      [t("author"), source.author],
                      [t("personalRole"), source.personal_role],
                      [
                        t("materialRole"),
                        old(
                          `materialRole.${["reference", "own-work", "team-work", "unspecified"].includes(source.material_role ?? "") ? source.material_role : "unspecified"}` as Parameters<
                            typeof old
                          >[0]
                        ),
                      ],
                      [
                        t("extractionScope"),
                        source.page_count
                          ? t("pages", { count: source.page_count })
                          : source.format === "unknown"
                            ? null
                            : source.format,
                      ],
                    ].map(([name, value]) => (
                      <div key={name} className="contents">
                        <dt className="text-muted-foreground">{name}</dt>
                        <dd className="min-w-0 break-words">{value || "—"}</dd>
                      </div>
                    ))}
                  </dl>
                  {source.eligibility !== "failed" &&
                    !!source.warnings?.length && (
                      <ul className="list-disc space-y-2 ps-5 text-sm">
                        {source.warnings.map((warning, index) => (
                          <li key={index}>{warning}</li>
                        ))}
                      </ul>
                    )}
                  <details className="rounded border p-3 text-xs text-muted-foreground">
                    <summary className="cursor-pointer">
                      {t("technicalDetails")}
                    </summary>
                    <pre className="mt-3 overflow-auto whitespace-pre-wrap break-all">
                      {JSON.stringify(
                        {
                          id: source.id,
                          path: source.raw_path,
                          hash: source.raw_hash,
                          revision: source.annotation_revision,
                          warnings: source.warnings,
                          read_error: readError,
                        },
                        null,
                        2
                      )}
                    </pre>
                  </details>
                </div>
              )}
              {activeTab === "related" &&
                (detail.data.related_notes.length ? (
                  <div className="space-y-2">
                    {detail.data.related_notes.map((note) => (
                      <WikiNoteCard
                        key={`${note.note_id}:${note.path}`}
                        note={note}
                        onClick={() => navigate({ path: note.path })}
                      />
                    ))}
                  </div>
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {t("noRelated")}
                  </p>
                ))}
            </>
          )}
        </article>
      </div>
    </div>
  )
}
