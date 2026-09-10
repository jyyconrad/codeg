"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import {
  computeStitchLayout,
  defaultGridColumns,
  type StitchMode,
} from "./image-stitch"
import { ImageResultPreview } from "./image-preview"
import { useObjectUrl } from "./use-object-url"
import {
  canvasToBlob,
  clampQuality,
  createExportCanvas,
  EXPORT_FORMATS,
  extForFormat,
  formatByteSize,
  guardImageFile,
  guardImageSize,
  IMAGE_DECODE_MESSAGE,
  loadImageFromUrl,
  makeSampleImageFile,
  mimeForFormat,
  TOOL_LABEL_CLASS,
  TOOL_SELECT_CLASS,
  triggerBlobDownload,
  withExtension,
  type ExportFormat,
} from "./image-io"

type Loaded = {
  file: File
  image: HTMLImageElement
}

async function loadAll(files: File[]): Promise<Loaded[]> {
  const out: Loaded[] = []
  for (const file of files) {
    const sizeErr = guardImageFile(file)
    if (sizeErr) throw new Error(sizeErr)
    const url = URL.createObjectURL(file)
    try {
      const image = await loadImageFromUrl(url)
      const dimErr = guardImageSize(image.naturalWidth, image.naturalHeight)
      if (dimErr) throw new Error(dimErr)
      out.push({ file, image })
    } finally {
      URL.revokeObjectURL(url)
    }
  }
  return out
}

export default function ImageStitchTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [files, setFiles] = useState<File[]>([])
  const [loaded, setLoaded] = useState<{
    names: string
    images: HTMLImageElement[]
  } | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [mode, setMode] = useState<StitchMode>("horizontal")
  const [gap, setGap] = useState(0)
  const [columns, setColumns] = useState(2)
  const [background, setBackground] = useState("#ffffff")
  const [format, setFormat] = useState<ExportFormat>("png")
  const [blob, setBlob] = useState<Blob | null>(null)
  const [exportError, setExportError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) setFiles([])
  }, [])
  useToolPendingInput(onInput)

  const fileKey = files.map((file) => `${file.name}:${file.size}`).join("|")
  const images = useMemo(
    () => (loaded?.names === fileKey ? loaded.images : []),
    [loaded, fileKey]
  )
  const resultBlob = images.length > 0 && !exportError ? blob : null
  const outUrl = useObjectUrl(resultBlob)

  useEffect(() => {
    if (files.length === 0) return
    let cancelled = false
    void loadAll(files)
      .then((next) => {
        if (cancelled) return
        setLoaded({
          names: next
            .map((item) => `${item.file.name}:${item.file.size}`)
            .join("|"),
          images: next.map((item) => item.image),
        })
        setLoadError(null)
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setLoadError(err instanceof Error ? err.message : IMAGE_DECODE_MESSAGE)
      })
    return () => {
      cancelled = true
    }
  }, [files])

  useEffect(() => {
    if (images.length === 0) return
    let cancelled = false
    void (async () => {
      try {
        const layout = computeStitchLayout(
          images.map((img) => ({
            width: img.naturalWidth,
            height: img.naturalHeight,
          })),
          mode,
          { gap, columns }
        )
        const dimErr = guardImageSize(layout.width, layout.height)
        if (dimErr) {
          if (!cancelled) setExportError(dimErr)
          return
        }
        const mime = mimeForFormat(format)
        const { canvas, ctx } = createExportCanvas(
          layout.width,
          layout.height,
          mime
        )
        ctx.fillStyle = background
        ctx.fillRect(0, 0, layout.width, layout.height)
        layout.slots.forEach((slot, index) => {
          ctx.drawImage(images[index], slot.x, slot.y, slot.width, slot.height)
        })
        const next = await canvasToBlob(
          canvas,
          mime,
          format === "png" ? undefined : clampQuality(0.92)
        )
        if (cancelled) return
        setBlob(next)
        setExportError(null)
      } catch (err) {
        if (cancelled) return
        setExportError(err instanceof Error ? err.message : "Export failed.")
      }
    })()
    return () => {
      cancelled = true
    }
  }, [images, mode, gap, columns, background, format])

  const summary = useMemo(() => {
    if (images.length === 0 || !resultBlob) return ""
    const layout = computeStitchLayout(
      images.map((img) => ({
        width: img.naturalWidth,
        height: img.naturalHeight,
      })),
      mode,
      { gap, columns }
    )
    return [
      `${images.length} → ${layout.width}×${layout.height}`,
      formatByteSize(resultBlob.size),
      mode,
    ].join("\n")
  }, [images, resultBlob, mode, gap, columns])

  function handleFiles(next: File[]) {
    setFiles(next)
    setInput(next.map((file) => file.name).join("\n"))
    if (next.length > 0) {
      setColumns(defaultGridColumns(next.length))
    }
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      inputLabel={t("input")}
      downloadFilename={withExtension("stitch", extForFormat(format))}
      error={loadError ?? exportError}
      result={summary}
      onExample={() => {
        void Promise.all([
          makeSampleImageFile(240, 160, "#2563eb", "A", "a.png"),
          makeSampleImageFile(180, 200, "#db2777", "B", "b.png"),
          makeSampleImageFile(200, 140, "#059669", "C", "c.png"),
        ]).then(handleFiles)
      }}
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <label className={TOOL_LABEL_CLASS}>
              Layout
              <select
                className={TOOL_SELECT_CLASS}
                value={mode}
                onChange={(event) => setMode(event.target.value as StitchMode)}
              >
                <option value="horizontal">horizontal</option>
                <option value="vertical">vertical</option>
                <option value="grid">grid</option>
              </select>
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Gap
              <Input
                type="number"
                min={0}
                className="h-8 w-20"
                value={gap}
                onChange={(event) =>
                  setGap(Math.max(0, Number(event.target.value) || 0))
                }
              />
            </label>
            {mode === "grid" ? (
              <label className={TOOL_LABEL_CLASS}>
                Columns
                <Input
                  type="number"
                  min={1}
                  className="h-8 w-20"
                  value={columns}
                  onChange={(event) =>
                    setColumns(Math.max(1, Number(event.target.value) || 1))
                  }
                />
              </label>
            ) : null}
            <label className={TOOL_LABEL_CLASS}>
              Fill
              <Input
                type="color"
                className="h-8 w-16 p-1"
                value={background}
                onChange={(event) => setBackground(event.target.value)}
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              JPEG / WebP / PNG
              <select
                className={TOOL_SELECT_CLASS}
                value={format}
                onChange={(event) =>
                  setFormat(event.target.value as ExportFormat)
                }
              >
                {EXPORT_FORMATS.map((item) => (
                  <option key={item} value={item}>
                    {item.toUpperCase()}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </div>
      }
      inputSlot={
        <Input
          key={input || "empty"}
          type="file"
          accept="image/*"
          multiple
          onChange={(event) =>
            handleFiles(Array.from(event.target.files ?? []))
          }
        />
      }
      resultSlot={
        <ImageResultPreview
          url={outUrl}
          meta={resultBlob ? formatByteSize(resultBlob.size) : undefined}
          downloadLabel={t("download")}
          onDownload={() => {
            if (!resultBlob) return
            triggerBlobDownload(
              resultBlob,
              withExtension("stitch", extForFormat(format))
            )
          }}
        />
      }
    />
  )
}
