"use client"

import { useCallback, useState } from "react"
import { FolderInput, Loader2, MessageSquarePlus } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import { DirectoryBrowserDialog } from "@/components/shared/directory-browser-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { toErrorMessage } from "@/lib/app-error"
import {
  getWikiSettings,
  scanImportableSessions,
  wikiImportDirectory,
  wikiImportLocalSessions,
} from "@/lib/api"
import {
  normalizeWikiSettings,
  type WikiBulkImportResult,
} from "@/lib/wiki-types"

function newRequestId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID()
  }
  return `wiki-import-${Date.now()}`
}

export function WikiImportButton() {
  const t = useTranslations("Wiki")
  const [open, setOpen] = useState(false)
  const [dirOpen, setDirOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const [scanCount, setScanCount] = useState<number | null>(null)

  const report = useCallback(
    (result: WikiBulkImportResult) => {
      toast.success(
        t("importResult", {
          imported: result.imported,
          duplicates: result.duplicates,
          failed: result.failed,
        })
      )
      if (result.errors && result.errors.length > 0) {
        toast.error(result.errors[0])
      }
    },
    [t]
  )

  const ensureEnabled = useCallback(async () => {
    const settings = normalizeWikiSettings(await getWikiSettings())
    if (!settings.enabled) {
      throw new Error(t("sources.disabledHint"))
    }
  }, [t])

  const handleOpen = useCallback(async () => {
    setOpen(true)
    setScanCount(null)
    try {
      const scan = await scanImportableSessions()
      setScanCount(scan.importable_count)
    } catch {
      setScanCount(null)
    }
  }, [])

  const handleImportSessions = useCallback(async () => {
    if (busy) return
    setBusy(true)
    try {
      await ensureEnabled()
      const result = await wikiImportLocalSessions({
        request_id: newRequestId(),
        all: true,
      })
      report(result)
      setOpen(false)
    } catch (err) {
      toast.error(t("importFailed", { message: toErrorMessage(err) }))
    } finally {
      setBusy(false)
    }
  }, [busy, ensureEnabled, report, t])

  const handleDirectory = useCallback(
    async (path: string) => {
      if (!path || busy) return
      setBusy(true)
      try {
        await ensureEnabled()
        const result = await wikiImportDirectory({
          request_id: newRequestId(),
          path,
        })
        report(result)
        setOpen(false)
      } catch (err) {
        toast.error(t("importFailed", { message: toErrorMessage(err) }))
      } finally {
        setBusy(false)
      }
    },
    [busy, ensureEnabled, report, t]
  )

  return (
    <>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={() => void handleOpen()}
      >
        <FolderInput className="size-3.5" />
        {t("import")}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("importTitle")}</DialogTitle>
            <DialogDescription>{t("importSessionsHint")}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-3">
            <div className="rounded-lg border p-3">
              <div className="text-sm font-medium">{t("importSessions")}</div>
              <p className="mt-1 text-xs text-muted-foreground">
                {scanCount == null
                  ? t("importSessionsHint")
                  : t("importSessionsCount", { count: scanCount })}
              </p>
              <Button
                type="button"
                size="sm"
                className="mt-3"
                disabled={busy}
                onClick={() => void handleImportSessions()}
              >
                {busy ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <MessageSquarePlus className="size-3.5" />
                )}
                {busy ? t("importing") : t("importSessionsAction")}
              </Button>
            </div>
            <div className="rounded-lg border p-3">
              <div className="text-sm font-medium">{t("importDirectory")}</div>
              <p className="mt-1 text-xs text-muted-foreground">
                {t("importDirectoryHint")}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="mt-3"
                disabled={busy}
                onClick={() => {
                  setOpen(false)
                  setDirOpen(true)
                }}
              >
                <FolderInput className="size-3.5" />
                {t("importDirectoryAction")}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
      <DirectoryBrowserDialog
        open={dirOpen}
        onOpenChange={setDirOpen}
        onSelect={(path) => void handleDirectory(path)}
        title={t("importDirectory")}
      />
    </>
  )
}
