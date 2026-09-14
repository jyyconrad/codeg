"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { toErrorMessage } from "@/lib/app-error"
import {
  getWikiSettings,
  wikiAcceptExtraction,
  wikiGetSource,
  wikiImportFiles,
  wikiImportText,
  wikiListSources,
  wikiUpdateSourceAnnotations,
  wikiVaultRead,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  normalizeWikiList,
  normalizeWikiSettings,
  wikiSourcePreviewPaths,
  wikiSourceTitle,
  wikiVaultReadContent,
  type WikiMaterialRole,
  type WikiImportBatchResult,
  type WikiImportResult,
  type WikiSource,
} from "@/lib/wiki-types"
import { WikiEmptyState, WikiMarkdownPreview } from "./wiki-shared"

const MAX_IMPORT_FILES = 20
const MAX_FILE_BYTES = 20 * 1024 * 1024
const MATERIAL_ROLES = [
  "reference",
  "own-work",
  "team-work",
  "unspecified",
] as const satisfies readonly WikiMaterialRole[]

function newRequestId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `req-${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function arrayBufferToBase64(bytes: Uint8Array): string {
  let binary = ""
  const chunk = 0x8000
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + chunk))
    )
  }
  return btoa(binary)
}

function isImportBatchResult(
  value: WikiImportResult | WikiImportBatchResult
): value is WikiImportBatchResult {
  return "results" in value && Array.isArray(value.results)
}

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
  const fileInputRef = useRef<HTMLInputElement>(null)
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
  const [wikiEnabled, setWikiEnabled] = useState(false)
  const [pasteText, setPasteText] = useState("")
  const [title, setTitle] = useState("")
  const [sourceUrl, setSourceUrl] = useState("")
  const [author, setAuthor] = useState("")
  const [materialRole, setMaterialRole] =
    useState<WikiMaterialRole>("reference")
  const [personalRole, setPersonalRole] = useState("")
  const [importing, setImporting] = useState(false)
  const [importMessage, setImportMessage] = useState<string | null>(null)
  const [importError, setImportError] = useState<string | null>(null)
  const [accepting, setAccepting] = useState(false)
  const [savingAnnotations, setSavingAnnotations] = useState(false)

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

  useEffect(() => {
    getWikiSettings()
      .then((raw) => {
        setWikiEnabled(normalizeWikiSettings(raw).enabled)
      })
      .catch(() => {
        setWikiEnabled(false)
      })
  }, [])

  const importMeta = useCallback(
    () => ({
      title: title.trim() || null,
      source_url: sourceUrl.trim() || null,
      author: author.trim() || null,
      material_role: materialRole,
      personal_role: personalRole.trim() || null,
    }),
    [author, materialRole, personalRole, sourceUrl, title]
  )

  const selectSource = useCallback(
    async (source: WikiSource) => {
      setSelectedId(source.id)
      if (
        source.material_role === "reference" ||
        source.material_role === "own-work" ||
        source.material_role === "team-work" ||
        source.material_role === "unspecified"
      ) {
        setMaterialRole(source.material_role)
      }
      setPersonalRole(source.personal_role ?? "")
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

  const afterImport = useCallback(
    async (source: WikiSource, duplicate: boolean) => {
      setSelectedId(source.id)
      setImportMessage(duplicate ? t("sources.duplicateNotice") : null)
      await load(0)
      await selectSource(source)
    },
    [load, selectSource, t]
  )

  const handlePasteImport = useCallback(async () => {
    const text = pasteText.trim()
    if (!text || importing || !wikiEnabled) return
    setImporting(true)
    setImportError(null)
    setImportMessage(null)
    try {
      const result = await wikiImportText({
        request_id: newRequestId(),
        text,
        ...importMeta(),
      })
      setPasteText("")
      await afterImport(result, result.duplicate)
    } catch (err) {
      setImportError(
        t("sources.importFailed", { message: toErrorMessage(err) })
      )
    } finally {
      setImporting(false)
    }
  }, [afterImport, importMeta, importing, pasteText, t, wikiEnabled])

  const handleFiles = useCallback(
    async (fileList: FileList | null) => {
      if (!fileList || fileList.length === 0 || importing || !wikiEnabled)
        return
      const files = Array.from(fileList).slice(0, MAX_IMPORT_FILES)
      setImporting(true)
      setImportError(null)
      setImportMessage(null)
      const errors: string[] = []
      const encodedFiles: Array<{
        filename: string
        mime: string | null
        bytes_base64: string
      }> = []
      for (const file of files) {
        if (file.size > MAX_FILE_BYTES) {
          errors.push(t("sources.fileTooLarge", { name: file.name }))
          continue
        }
        const bytes = new Uint8Array(await file.arrayBuffer())
        encodedFiles.push({
          filename: file.name,
          mime: file.type || null,
          bytes_base64: arrayBufferToBase64(bytes),
        })
      }
      let last: WikiSource | null = null
      let lastDuplicate = false
      if (encodedFiles.length > 0) {
        try {
          const result = await wikiImportFiles({
            request_id: newRequestId(),
            files: encodedFiles,
            ...importMeta(),
          })
          if (isImportBatchResult(result)) {
            const successful = result.results.filter((item) => item.source)
            const selected = successful[successful.length - 1]
            if (selected?.source) {
              last = selected.source
              lastDuplicate = selected.duplicate
            }
            errors.push(
              ...result.results
                .filter((item) => item.error)
                .map((item) => `${item.filename}: ${item.error}`)
            )
          } else {
            last = result
            lastDuplicate = result.duplicate
          }
        } catch (err) {
          errors.push(toErrorMessage(err))
        }
      }
      if (fileInputRef.current) fileInputRef.current.value = ""
      if (last) await afterImport(last, lastDuplicate)
      if (errors.length > 0) {
        setImportError(
          t("sources.importFailed", { message: errors.join(" · ") })
        )
      }
      setImporting(false)
    },
    [afterImport, importMeta, importing, t, wikiEnabled]
  )

  const handleAccept = useCallback(async () => {
    if (!selectedId || accepting) return
    setAccepting(true)
    setImportError(null)
    try {
      const updated = await wikiAcceptExtraction(selectedId)
      setSources((current) =>
        current.map((row) =>
          row.id === updated.id ? { ...row, ...updated } : row
        )
      )
    } catch (err) {
      setImportError(
        t("sources.importFailed", { message: toErrorMessage(err) })
      )
    } finally {
      setAccepting(false)
    }
  }, [accepting, selectedId, t])

  const handleSaveAnnotations = useCallback(async () => {
    if (!selectedId || savingAnnotations) return
    setSavingAnnotations(true)
    setImportError(null)
    try {
      const updated = await wikiUpdateSourceAnnotations({
        source_id: selectedId,
        material_role: materialRole,
        personal_role: personalRole.trim() || null,
      })
      setSources((current) =>
        current.map((row) =>
          row.id === updated.id ? { ...row, ...updated } : row
        )
      )
    } catch (err) {
      setImportError(
        t("sources.importFailed", { message: toErrorMessage(err) })
      )
    } finally {
      setSavingAnnotations(false)
    }
  }, [materialRole, personalRole, savingAnnotations, selectedId, t])

  const selected = sources.find((row) => row.id === selectedId) ?? null
  const selectedEligibility = eligibilityKey(selected?.eligibility)
  const importDisabled = !wikiEnabled || importing

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WikiEmptyState
        title={t("sources.emptyTitle")}
        description={t("sources.emptyDescription")}
      >
        <div className="space-y-3">
          <div className="grid gap-2 sm:grid-cols-2">
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder={t("sources.titlePlaceholder")}
              disabled={importDisabled}
            />
            <Input
              value={sourceUrl}
              onChange={(e) => setSourceUrl(e.target.value)}
              placeholder={t("sources.sourceUrlPlaceholder")}
              disabled={importDisabled}
            />
            <Input
              value={author}
              onChange={(e) => setAuthor(e.target.value)}
              placeholder={t("sources.authorPlaceholder")}
              disabled={importDisabled}
            />
            <Input
              value={personalRole}
              onChange={(e) => setPersonalRole(e.target.value)}
              placeholder={t("sources.personalRolePlaceholder")}
              disabled={importDisabled}
            />
          </div>
          <Select
            value={materialRole}
            onValueChange={(value) =>
              setMaterialRole((value || "reference") as WikiMaterialRole)
            }
            disabled={importDisabled}
          >
            <SelectTrigger className="w-full" size="sm">
              <SelectValue placeholder={t("sources.materialRoleLabel")} />
            </SelectTrigger>
            <SelectContent>
              {MATERIAL_ROLES.map((role) => (
                <SelectItem key={role} value={role}>
                  {t(`sources.materialRole.${role}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept=".md,.markdown,.txt,.pdf,.docx,text/markdown,text/plain,application/pdf,application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            className="hidden"
            disabled={importDisabled}
            onChange={(e) => {
              handleFiles(e.target.files).catch(console.error)
            }}
          />
          <div className="flex flex-wrap gap-2">
            <Button
              type="button"
              size="sm"
              disabled={importDisabled}
              onClick={() => fileInputRef.current?.click()}
            >
              {importing ? t("sources.importing") : t("sources.importFiles")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={importDisabled || !pasteText.trim()}
              onClick={() => {
                handlePasteImport().catch(console.error)
              }}
            >
              {t("sources.importPaste")}
            </Button>
          </div>
          <Textarea
            disabled={importDisabled}
            value={pasteText}
            onChange={(e) => setPasteText(e.target.value)}
            placeholder={t("sources.pastePlaceholder")}
            className="min-h-20"
          />
          <p className="text-xs leading-5 text-muted-foreground">
            {wikiEnabled ? t("sources.importHint") : t("sources.disabledHint")}
          </p>
          <p className="text-xs leading-5 text-muted-foreground">
            {t("sources.privacyHint")}
          </p>
          {importMessage ? (
            <p className="text-xs text-muted-foreground">{importMessage}</p>
          ) : null}
          {importError ? (
            <p className="text-xs text-destructive">{importError}</p>
          ) : null}
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
            {selected && selectedEligibility === "awaiting-acceptance" ? (
              <div className="mb-4 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-sm">
                <p className="mb-2">{t("sources.partialBanner")}</p>
                <Button
                  type="button"
                  size="sm"
                  disabled={accepting}
                  onClick={() => {
                    handleAccept().catch(console.error)
                  }}
                >
                  {accepting
                    ? t("sources.accepting")
                    : t("sources.acceptExtraction")}
                </Button>
              </div>
            ) : null}
            {selected?.warnings && selected.warnings.length > 0 ? (
              <div className="mb-4 rounded-lg border p-3 text-xs leading-5">
                <p className="mb-1 font-medium">{t("sources.warningsTitle")}</p>
                <ul className="list-disc space-y-1 pl-4">
                  {selected.warnings.map((warning) => (
                    <li key={warning}>{warning}</li>
                  ))}
                </ul>
              </div>
            ) : null}
            {selected ? (
              <div className="mb-4 flex flex-wrap items-end gap-2">
                <Select
                  value={materialRole}
                  onValueChange={(value) =>
                    setMaterialRole((value || "reference") as WikiMaterialRole)
                  }
                >
                  <SelectTrigger className="w-44" size="sm">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {MATERIAL_ROLES.map((role) => (
                      <SelectItem key={role} value={role}>
                        {t(`sources.materialRole.${role}`)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={savingAnnotations}
                  onClick={() => {
                    handleSaveAnnotations().catch(console.error)
                  }}
                >
                  {t("sources.saveAnnotations")}
                </Button>
              </div>
            ) : null}
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
