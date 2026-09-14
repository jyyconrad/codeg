/**
 * 个人 Wiki 素材导入入口，支持文件、粘贴内容、指定目录及本地会话。
 * 专用 API 负责登记与提取，组件展示逐项结果和失败信息；本地会话扫描复用通用目录能力。
 */
"use client"

import { useState } from "react"
import { Plus } from "lucide-react"
import { useTranslations } from "next-intl"
import { DirectoryBrowserDialog } from "@/components/shared/directory-browser-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop } from "@/lib/platform"
import { scanImportableSessions } from "@/lib/api"
import {
  wikiImportDirectory,
  wikiImportLocalSessions,
  wikiImportFiles,
  wikiImportText,
} from "@/lib/wiki-api"
import type {
  WikiBulkImportResult,
  WikiImportFileResult,
  WikiSettingsView,
  WikiSource,
} from "@/lib/wiki-types"
import { useWikiData, useWikiQuery } from "./wiki-data"

function isPartialSource(source: WikiSource | null | undefined): boolean {
  return (
    source?.eligibility !== "failed" &&
    (source?.extraction_status === "partial" ||
      source?.eligibility === "awaiting-acceptance")
  )
}

export function WikiImportButton() {
  const t = useTranslations("Wiki.v2")
  const old = useTranslations("Wiki")
  const { invalidate, navigate } = useWikiData()
  const [open, setOpen] = useState(false)
  const [mode, setMode] = useState("files")
  const [dirOpen, setDirOpen] = useState(false)
  const [path, setPath] = useState("")
  const [files, setFiles] = useState<File[]>([])
  const [text, setText] = useState("")
  const [title, setTitle] = useState("")
  const [author, setAuthor] = useState("")
  const [role, setRole] = useState("reference")
  const [personalRole, setPersonalRole] = useState("")
  const [sourceUrl, setSourceUrl] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [rows, setRows] = useState<WikiImportFileResult[]>([])
  const [counts, setCounts] = useState<{
    imported: number
    duplicates: number
    failed: number
    partial: number
  } | null>(null)
  const [scanCount, setScanCount] = useState<number | null>(null)
  const settings = useWikiQuery<WikiSettingsView>("get_wiki_settings", {}, open)
  const meta = {
    title: title.trim() || null,
    author: author.trim() || null,
    material_role: role,
    personal_role: personalRole.trim() || null,
    source_url: sourceUrl.trim() || null,
  }
  const handleOpen = () => {
    setOpen(true)
    setError(null)
  }
  const chooseMode = (value: string) => {
    setMode(value)
    setError(null)
    if (value === "history" && scanCount == null)
      scanImportableSessions()
        .then((result) => setScanCount(result.importable_count))
        .catch((err) => setError(toErrorMessage(err)))
  }
  const importBulk = (result: WikiBulkImportResult) => {
    setRows(result.results ?? [])
    setCounts({
      ...result,
      partial:
        result.partial ??
        result.results?.filter(
          (item) => item.status === "succeeded" && isPartialSource(item.source)
        ).length ??
        0,
    })
    if (result.errors?.length) setError(result.errors.join("\n"))
  }
  const submit = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    setRows([])
    setCounts(null)
    const request_id = crypto.randomUUID()
    try {
      if (mode === "text") {
        const source = await wikiImportText({ request_id, text, ...meta })
        const failed = source.eligibility === "failed"
        const duplicate = !failed && source.duplicate
        setRows([
          {
            filename: source.source_title || title || t("untitled"),
            request_id,
            source,
            duplicate,
            status: failed ? "failed" : duplicate ? "duplicate" : "succeeded",
            error: failed
              ? source.warnings?.filter(Boolean).join(" · ") ||
                t("noExtractedText")
              : undefined,
          },
        ])
        setCounts({
          imported: failed || duplicate ? 0 : 1,
          duplicates: duplicate ? 1 : 0,
          failed: failed ? 1 : 0,
          partial: !failed && !duplicate && isPartialSource(source) ? 1 : 0,
        })
      } else if (mode === "files") {
        const incoming: WikiImportFileResult[] = []
        for (const file of files) {
          const itemId = crypto.randomUUID()
          if (file.size > 20 * 1024 * 1024) {
            incoming.push({
              filename: file.name,
              request_id: itemId,
              duplicate: false,
              status: "failed",
              error: old("sources.fileTooLarge", { name: file.name }),
            })
            continue
          }
          try {
            const bytes = new Uint8Array(await file.arrayBuffer())
            let binary = ""
            for (let index = 0; index < bytes.length; index += 8192)
              binary += String.fromCharCode(
                ...bytes.subarray(index, index + 8192)
              )
            const result = await wikiImportFiles({
              request_id: itemId,
              files: [
                {
                  filename: file.name,
                  mime: file.type,
                  bytes_base64: btoa(binary),
                },
              ],
              ...meta,
            })
            incoming.push(...result.results)
          } catch (err) {
            incoming.push({
              filename: file.name,
              request_id: itemId,
              duplicate: false,
              status: "failed",
              error: toErrorMessage(err),
            })
          }
          setRows([...incoming])
        }
        setRows(incoming)
        setCounts({
          imported: incoming.filter((item) => item.status === "succeeded")
            .length,
          duplicates: incoming.filter((item) => item.status === "duplicate")
            .length,
          failed: incoming.filter((item) => item.status === "failed").length,
          partial: incoming.filter(
            (item) =>
              item.status === "succeeded" && isPartialSource(item.source)
          ).length,
        })
      } else if (mode === "history")
        importBulk(await wikiImportLocalSessions({ request_id, all: true }))
      else importBulk(await wikiImportDirectory({ request_id, path }))
    } catch (err) {
      setError(toErrorMessage(err))
    } finally {
      invalidate()
      setBusy(false)
    }
  }
  const missing =
    mode === "files"
      ? !files.length || files.length > 20
      : mode === "text"
        ? !text.trim()
        : mode === "directory"
          ? !path.trim()
          : scanCount === 0
  return (
    <>
      <Button type="button" variant="outline" size="sm" onClick={handleOpen}>
        <Plus className="size-4" />
        {t("addSources")}
      </Button>
      <Dialog
        open={open}
        onOpenChange={(value) => {
          setOpen(value)
          if (!value) invalidate()
        }}
      >
        <DialogContent className="max-h-[90dvh] overflow-y-auto sm:max-w-xl">
          <DialogHeader>
            <DialogTitle>{t("addSources")}</DialogTitle>
            <DialogDescription>{t("archiveHint")}</DialogDescription>
          </DialogHeader>
          <div
            className="flex flex-wrap gap-1"
            role="group"
            aria-label={t("importMethods")}
          >
            {(["files", "text", "history", "directory"] as const).map(
              (value) => (
                <Button
                  type="button"
                  key={value}
                  size="sm"
                  variant={mode === value ? "secondary" : "ghost"}
                  aria-pressed={mode === value}
                  disabled={busy}
                  onClick={() => chooseMode(value)}
                >
                  {t(`importModes.${value}`)}
                </Button>
              )
            )}
          </div>
          <fieldset disabled={busy} className="space-y-4">
            {mode === "files" && (
              <label className="block space-y-2 text-sm">
                <span>{t("chooseFiles")}</span>
                <Input
                  type="file"
                  multiple
                  accept=".md,.markdown,.txt,.pdf,.docx"
                  onChange={(event) =>
                    setFiles(Array.from(event.target.files ?? []))
                  }
                />
                <span className="block text-xs leading-5 text-muted-foreground">
                  {old("sources.importHint")}
                </span>
              </label>
            )}
            {mode === "text" && (
              <label className="block space-y-2 text-sm">
                <span>{t("pasteText")}</span>
                <Textarea
                  value={text}
                  onChange={(event) => setText(event.target.value)}
                  className="min-h-36"
                  placeholder={old("sources.pastePlaceholder")}
                />
              </label>
            )}
            {mode === "history" && (
              <div className="rounded-lg border p-4 text-sm">
                <p>{old("importSessionsHint")}</p>
                {scanCount != null && (
                  <p className="mt-3 font-medium">
                    {old("importSessionsCount", { count: scanCount })}
                  </p>
                )}
              </div>
            )}
            {mode === "directory" && (
              <div className="space-y-3">
                <p className="text-sm">
                  {isLocalDesktop()
                    ? old("importDirectoryHint")
                    : t("serverDirectoryHint")}
                </p>
                <p className="break-all text-sm text-muted-foreground">
                  {path}
                </p>
                <Button
                  size="sm"
                  type="button"
                  variant="outline"
                  onClick={() => setDirOpen(true)}
                >
                  {old("importDirectoryAction")}
                </Button>
              </div>
            )}
            {(mode === "files" || mode === "text") && (
              <>
                <label className="block space-y-2 text-sm">
                  <span>{t("optionalTitle")}</span>
                  <Input
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                  />
                </label>
                <details className="rounded-lg border p-3 text-sm">
                  <summary className="cursor-pointer">{t("extraInfo")}</summary>
                  <div className="mt-3 grid gap-3 sm:grid-cols-2">
                    <label className="space-y-1">
                      <span>{t("author")}</span>
                      <Input
                        value={author}
                        onChange={(event) => setAuthor(event.target.value)}
                      />
                    </label>
                    <label className="space-y-1">
                      <span>{t("personalRole")}</span>
                      <Input
                        value={personalRole}
                        onChange={(event) =>
                          setPersonalRole(event.target.value)
                        }
                      />
                    </label>
                    <label className="space-y-1">
                      <span>{t("materialRole")}</span>
                      <Select value={role} onValueChange={setRole}>
                        <SelectTrigger className="w-full">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent align="start">
                          {(
                            [
                              "reference",
                              "own-work",
                              "team-work",
                              "unspecified",
                            ] as const
                          ).map((value) => (
                            <SelectItem key={value} value={value}>
                              {old(`sources.materialRole.${value}`)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </label>
                    <label className="space-y-1">
                      <span>{t("sourceUrl")}</span>
                      <Input
                        type="url"
                        value={sourceUrl}
                        onChange={(event) => setSourceUrl(event.target.value)}
                      />
                    </label>
                  </div>
                </details>
              </>
            )}
          </fieldset>
          {(error || settings.error) && (
            <p
              role="alert"
              className="whitespace-pre-wrap break-words text-sm text-destructive"
            >
              {error || settings.error}
            </p>
          )}
          {settings.data && !settings.data.enabled && (
            <p className="text-sm text-muted-foreground">
              {old("sources.disabledHint")}
            </p>
          )}
          {counts && (
            <p role="status" className="text-sm font-medium">
              {t("importResult", {
                ...counts,
                failed: counts.failed + counts.partial,
              })}
            </p>
          )}
          {!!counts?.partial && (
            <p className="text-sm text-muted-foreground">
              {t("partial")} · {counts.partial}
            </p>
          )}
          {!!rows.length && (
            <ul className="max-h-52 space-y-3 overflow-y-auto rounded-lg border p-3">
              {rows.map((row, index) => (
                <li key={`${row.request_id}:${index}`} className="text-sm">
                  <div className="flex items-start justify-between gap-3">
                    <span className="min-w-0 break-words">
                      {row.filename}
                      <span className="ms-2 text-muted-foreground">
                        {t(`importStatus.${row.status}`)}
                      </span>
                    </span>
                    {row.source && (
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => {
                          navigate({
                            view: "sources",
                            source: row.source!.id,
                            path: null,
                            job: null,
                            query: "",
                          })
                          setOpen(false)
                        }}
                      >
                        {t(
                          row.status === "failed" ? "sourceTabs.info" : "read"
                        )}
                      </Button>
                    )}
                  </div>
                  {row.status !== "failed" && isPartialSource(row.source) && (
                    <p className="mt-1 text-sm text-muted-foreground">
                      {t("partialHint")}
                    </p>
                  )}
                  {row.error &&
                    (row.source?.eligibility === "failed" ? (
                      <div className="mt-1 space-y-2 text-sm">
                        <p className="text-destructive">
                          {t("noExtractedText")}
                        </p>
                        <details className="text-muted-foreground">
                          <summary className="cursor-pointer">
                            {t("technicalDetails")}
                          </summary>
                          <p className="mt-1 break-words">{row.error}</p>
                        </details>
                      </div>
                    ) : (
                      <p className="mt-1 break-words text-destructive">
                        {row.error}
                      </p>
                    ))}
                </li>
              ))}
            </ul>
          )}
          <p className="border-t pt-3 text-xs leading-5 text-muted-foreground">
            {t("archiveHint")}
          </p>
          <div className="flex justify-end gap-2">
            {counts && (
              <Button
                variant="outline"
                onClick={() => {
                  navigate({
                    view: "sources",
                    path: null,
                    source: null,
                    query: "",
                  })
                  setOpen(false)
                }}
              >
                {t("viewSources")}
              </Button>
            )}
            <Button
              disabled={busy || missing || !settings.data?.enabled}
              onClick={() => void submit()}
            >
              {busy
                ? t("adding")
                : mode === "history"
                  ? old("importSessionsAction")
                  : t("addSources")}
            </Button>
          </div>
        </DialogContent>
      </Dialog>
      <DirectoryBrowserDialog
        open={dirOpen}
        onOpenChange={setDirOpen}
        onSelect={(value) => {
          setPath(value)
          setDirOpen(false)
        }}
        title={isLocalDesktop() ? old("importDirectory") : t("serverDirectory")}
      />
    </>
  )
}
