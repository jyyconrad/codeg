"use client"

import { useEffect, useRef, useState } from "react"
import { useTranslations } from "next-intl"
import { FileWarning, Loader2 } from "lucide-react"

import { usePreviewFileChanges } from "@/hooks/use-preview-file-changes"
import { readOfficeFileBytesWithRetry } from "@/lib/office-file-bytes"
import { extractAppCommandError } from "@/lib/app-error"

type JsPreviewer = {
  preview: (src: ArrayBuffer | string | Blob) => Promise<unknown>
  destroy: () => void
}

type OfficeJsKind = "pdf" | "docx" | "pptx"

async function createPreviewer(
  kind: OfficeJsKind,
  container: HTMLElement
): Promise<JsPreviewer> {
  if (kind === "pdf") {
    const mod = await import("@js-preview/pdf")
    return mod.default.init(container, {
      staticFileUrl: "/",
      useSystemFonts: true,
    })
  }
  if (kind === "docx") {
    await import("@js-preview/docx/lib/index.css")
    const mod = await import("@js-preview/docx")
    return mod.default.init(container)
  }
  const mod = await import("pptx-preview")
  return mod.init(container, { mode: "list" }) as JsPreviewer
}

export function OfficeJsPreview({
  kind,
  path,
  rootPath,
  relPath,
}: {
  kind: OfficeJsKind
  path: string
  rootPath: string | null
  relPath: string | null
}) {
  const t = useTranslations("Folder.fileWorkspacePanel")
  const containerRef = useRef<HTMLDivElement | null>(null)
  const previewerRef = useRef<JsPreviewer | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [reloadKey, setReloadKey] = useState(0)

  usePreviewFileChanges(path, () => setReloadKey((k) => k + 1))

  useEffect(() => {
    let cancelled = false
    const container = containerRef.current
    if (!container) return

    async function run() {
      setLoading(true)
      setError(null)
      try {
        const bytes = await readOfficeFileBytesWithRetry(rootPath, relPath, path)
        if (cancelled || !container) return
        previewerRef.current?.destroy()
        previewerRef.current = null
        container.replaceChildren()
        const previewer = await createPreviewer(kind, container)
        if (cancelled) {
          previewer.destroy()
          return
        }
        previewerRef.current = previewer
        await previewer.preview(bytes)
        if (!cancelled) setLoading(false)
      } catch (err) {
        if (cancelled) return
        setLoading(false)
        setError(
          extractAppCommandError(err)?.message ?? t("officeJsPreviewFailed")
        )
      }
    }

    void run()
    return () => {
      cancelled = true
      previewerRef.current?.destroy()
      previewerRef.current = null
    }
  }, [kind, path, rootPath, relPath, reloadKey, t])

  return (
    <div className="relative h-full min-h-0">
      {error ? (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-3 bg-background px-6 text-center">
          <FileWarning className="h-8 w-8 text-muted-foreground" />
          <div className="text-sm font-medium text-foreground">
            {t("officeJsPreviewFailed")}
          </div>
          <div className="max-w-sm break-words text-xs text-muted-foreground">
            {error}
          </div>
          <button
            type="button"
            onClick={() => setReloadKey((k) => k + 1)}
            className="mt-1 rounded-md border border-border bg-card px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-primary/8"
          >
            {t("officeJsPreviewRetry")}
          </button>
        </div>
      ) : loading ? (
        <div className="absolute inset-0 z-10 flex items-center justify-center gap-2 bg-background/60 text-xs text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("loading")}
        </div>
      ) : null}
      <div ref={containerRef} className="h-full min-h-0 overflow-auto" />
    </div>
  )
}
