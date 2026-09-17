/**
 * Wiki 笔记摘要与正文阅读组件，展示 Markdown、目录锚点、原文和来源地址。
 * 通过共享查询读取文档，跳转前探测目标可读性；按后端提供的来源存在状态控制打开操作。
 */
"use client"

import { useEffect, useRef, useState } from "react"
import { useTranslations } from "next-intl"
import { ArrowLeft, Loader2 } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import {
  wikiBodyForReading,
  wikiHeadingId,
  wikiSourceLineExcerpt,
} from "@/lib/wiki-content"
import { wikiReadNote } from "@/lib/wiki-api"
import type { WikiNoteDocument, WikiNoteSummary } from "@/lib/wiki-types"
import { useWikiData, useWikiQuery } from "./wiki-data"
import { WikiMarkdownPreview } from "./wiki-shared"

export function WikiNoteType({ type }: { type: string }) {
  const t = useTranslations("Wiki.v2")
  const key = `types.${type}` as Parameters<typeof t>[0]
  return <>{t.has(key) ? t(key) : t("types.note")}</>
}
export function WikiDate({ value }: { value?: string | null }) {
  if (!value) return null
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? null : (
    <time dateTime={value}>
      {date.toLocaleString(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      })}
    </time>
  )
}
export function WikiNoteCard({
  note,
  onClick,
  selected = false,
  projectNames = [],
}: {
  note: WikiNoteSummary
  onClick: () => void
  selected?: boolean
  projectNames?: string[]
}) {
  const t = useTranslations("Wiki.v2")
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={selected ? "page" : undefined}
      className={`w-full rounded-lg border p-4 text-start transition-colors hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-ring ${selected ? "border-primary/40 bg-muted" : "border-transparent"}`}
    >
      <span className="block break-words text-sm font-semibold">
        {note.title || t("untitled")}
      </span>
      {(note.excerpt || note.summary) && (
        <span className="mt-1 line-clamp-2 block text-sm leading-6 text-muted-foreground">
          {note.excerpt || note.summary}
        </span>
      )}
      <span className="mt-2 flex flex-wrap gap-x-2 gap-y-1 text-xs text-muted-foreground">
        <WikiNoteType type={note.type} />
        {projectNames.length > 0 && <span>{projectNames.join(" · ")}</span>}
        <WikiDate value={note.updated_at} />
      </span>
    </button>
  )
}

export function WikiNoteReader({
  path,
  homePath,
  refreshRevision,
}: {
  path: string
  homePath?: string
  refreshRevision?: number
}) {
  const t = useTranslations("Wiki.v2")
  const { navigate } = useWikiData()
  const { data, error, loading, refresh } = useWikiQuery<WikiNoteDocument>(
    "wiki_read_note",
    { path },
    true,
    refreshRevision
  )
  const [sourceMode, setSourceMode] = useState(false)
  const [linkError, setLinkError] = useState(false)
  const linkRequest = useRef({ generation: 0 })
  const [anchor, setAnchor] = useState(() =>
    typeof window === "undefined" ? "" : window.location.hash
  )
  const excerpt = data ? wikiSourceLineExcerpt(data.source, anchor) : null
  useEffect(() => {
    const changed = () => setAnchor(window.location.hash)
    window.addEventListener("hashchange", changed)
    return () => window.removeEventListener("hashchange", changed)
  }, [])
  useEffect(() => {
    const request = linkRequest.current
    return () => {
      request.generation++
    }
  }, [path])
  const openLink = async (next: string, anchor: string) => {
    const request = ++linkRequest.current.generation
    setLinkError(false)
    try {
      if (next !== path) await wikiReadNote(next)
      if (request !== linkRequest.current.generation) return
      if (next !== path) navigate({ path: next, source: null })
      window.location.hash = anchor ? encodeURIComponent(anchor) : ""
      setAnchor(anchor)
    } catch {
      if (request === linkRequest.current.generation) setLinkError(true)
    }
  }
  useEffect(() => {
    if (data && window.location.hash) {
      try {
        document
          .getElementById(decodeURIComponent(window.location.hash.slice(1)))
          ?.scrollIntoView({ block: "start" })
      } catch {}
    }
  }, [data, anchor])
  return (
    <article className="mx-auto w-full min-w-0 max-w-[840px] p-4 pb-12 sm:p-6">
      <div className="mb-5 flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="ghost"
          onClick={() =>
            navigate(
              homePath
                ? { view: "library", path: null, query: "" }
                : { path: null }
            )
          }
        >
          <ArrowLeft className="size-4 rtl:rotate-180" />
          {homePath ? t("library.backHome") : t("backToList")}
        </Button>
        <div className="ms-auto flex gap-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={refresh}
            aria-label={t("refresh")}
          >
            {loading ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              t("refresh")
            )}
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => setSourceMode((value) => !value)}
            aria-pressed={sourceMode}
          >
            {sourceMode ? t("read") : t("sourceCode")}
          </Button>
        </div>
      </div>
      {error && (
        <div
          role="alert"
          className="mb-4 space-y-2 rounded-lg border border-destructive/30 p-3 text-sm"
        >
          <p>
            {t("readFailed")}: {error}
          </p>
          <Button variant="outline" size="sm" onClick={refresh}>
            {t("retry")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => navigate({ view: "sources", path: null })}
          >
            {t("viewSources")}
          </Button>
        </div>
      )}
      {!data && loading && <p role="status">{t("loading")}</p>}
      {data && (
        <>
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <Badge variant="secondary">
              <WikiNoteType type={data.note.type} />
            </Badge>
            {data.note.evidence_level && (
              <Badge variant="outline">
                {t.has(
                  `evidence.${data.note.evidence_level}` as Parameters<
                    typeof t
                  >[0]
                )
                  ? t(
                      `evidence.${data.note.evidence_level}` as Parameters<
                        typeof t
                      >[0]
                    )
                  : t("evidence.reference")}
              </Badge>
            )}
          </div>
          <h1
            id={wikiHeadingId(data.note.title)}
            className="text-2xl font-semibold leading-tight break-words"
          >
            {data.note.title || t("untitled")}
          </h1>
          {data.note.summary && data.note.type !== "index" && (
            <p className="mt-3 text-base leading-7 text-muted-foreground">
              {data.note.summary}
            </p>
          )}
          <div className="my-4 text-xs text-muted-foreground">
            <WikiDate value={data.note.updated_at} />
          </div>
          {data.format_warning && (
            <p role="status" className="mb-4 rounded border p-3 text-sm">
              {t("formatWarning")}
            </p>
          )}
          {linkError && (
            <div role="alert" className="my-4 rounded border p-3 text-sm">
              {t("missingLink")}{" "}
              <a href="#wiki-note-sources" className="text-primary underline">
                {t("viewSources")}
              </a>
            </div>
          )}
          {!sourceMode &&
            data.note.type !== "index" &&
            data.headings.length > 2 && (
              <details className="my-5 rounded-lg border p-3 text-sm">
                <summary className="cursor-pointer font-medium">
                  {t("contents")}
                </summary>
                <ul className="mt-2 space-y-2">
                  {data.headings.map((heading) => (
                    <li
                      key={heading.id}
                      style={{
                        paddingInlineStart: Math.max(0, heading.level - 1) * 12,
                      }}
                    >
                      <a
                        href={`#${encodeURIComponent(heading.id)}`}
                        className="text-muted-foreground hover:text-foreground"
                      >
                        {heading.title}
                      </a>
                    </li>
                  ))}
                </ul>
              </details>
            )}
          {excerpt && (
            <section
              id={anchor.replace(/^#/, "").toLowerCase()}
              className="my-5 rounded-lg border border-primary/30 bg-muted/30 p-4"
            >
              <h2 className="mb-2 text-sm font-medium">
                {t("lines", { start: excerpt.start, end: excerpt.end })}
              </h2>
              <pre className="whitespace-pre-wrap break-words text-sm leading-6">
                {excerpt.text}
              </pre>
              <Button
                size="sm"
                variant="ghost"
                className="mt-3"
                onClick={() => setSourceMode(true)}
              >
                {t("sourceCode")}
              </Button>
            </section>
          )}
          <div className="border-t pt-4">
            {sourceMode ? (
              <pre className="max-w-full overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-4 font-mono text-xs leading-6">
                {data.source}
              </pre>
            ) : (
              <WikiMarkdownPreview
                content={wikiBodyForReading(data.body, data.note.title)}
                path={data.note.path}
                onOpenWikilink={(next, anchor) => void openLink(next, anchor)}
              />
            )}
          </div>
          <section
            id="wiki-note-sources"
            className="mt-8 scroll-mt-4 border-t pt-5"
          >
            <h2 className="mb-3 text-base font-semibold">{t("viewSources")}</h2>
            {data.sources.length ? (
              <ul className="space-y-3">
                {data.sources.map((source, index) => {
                  const dump = source.path.startsWith("raw/")
                  return (
                    <li key={`${source.path}:${index}`}>
                      <button
                        type="button"
                        className="text-start text-sm text-primary underline disabled:cursor-not-allowed disabled:text-muted-foreground disabled:no-underline"
                        disabled={source.availability === "missing"}
                        onClick={() => {
                          const anchor = source.start_line
                            ? `L${source.start_line}-L${source.end_line ?? source.start_line}`
                            : ""
                          if (source.source_id) {
                            navigate({
                              view: "sources",
                              source: source.source_id,
                              path: null,
                            })
                            if (anchor) window.location.hash = anchor
                          } else if (!dump) void openLink(source.path, anchor)
                        }}
                      >
                        {source.title || t("untitled")}
                      </button>
                      {source.availability === "missing" && (
                        <Badge variant="outline" className="ms-2">
                          {t(`sourceAvailability.${source.availability}`)}
                        </Badge>
                      )}
                      {!dump && source.path && (
                        <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                          {source.path}
                        </p>
                      )}
                      {!dump &&
                        source.source_url &&
                        source.source_url !== source.path && (
                          <p className="mt-1 break-all text-xs text-muted-foreground">
                            {source.source_url}
                          </p>
                        )}
                    </li>
                  )
                })}
              </ul>
            ) : (
              <p className="text-sm text-muted-foreground">{t("noSources")}</p>
            )}
          </section>
          <details className="mt-6 text-xs text-muted-foreground">
            <summary className="cursor-pointer">{t("fileLocation")}</summary>
            <p className="mt-2 break-all font-mono">{data.note.path}</p>
          </details>
        </>
      )}
    </article>
  )
}
