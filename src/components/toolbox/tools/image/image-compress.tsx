"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { ImageResultPreview } from "./image-preview"
import { useLoadedImage } from "./use-loaded-image"
import { useObjectUrl } from "./use-object-url"
import {
  clampQuality,
  computeScaledSize,
  EXPORT_FORMATS,
  extForFormat,
  formatByteSize,
  makeSampleImageFile,
  mimeForFormat,
  blobFromCanvasImage,
  TOOL_LABEL_CLASS,
  triggerBlobDownload,
  withExtension,
  type ExportFormat,
} from "./image-io"

export default function ImageCompressTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [file, setFile] = useState<File | null>(null)
  const [quality, setQuality] = useState(0.8)
  const [maxWidth, setMaxWidth] = useState(1920)
  const [format, setFormat] = useState<ExportFormat>("jpeg")
  const [blob, setBlob] = useState<Blob | null>(null)
  const [exportError, setExportError] = useState<string | null>(null)
  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) setFile(null)
  }, [])
  useToolPendingInput(onInput)

  const { image, error: loadError } = useLoadedImage(file)

  useEffect(() => {
    if (!image) return
    let cancelled = false
    const size = computeScaledSize(
      image.naturalWidth,
      image.naturalHeight,
      maxWidth
    )
    void blobFromCanvasImage(
      image,
      size.width,
      size.height,
      format,
      clampQuality(quality)
    )
      .then((next) => {
        if (cancelled) return
        setBlob(next)
        setExportError(null)
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setBlob(null)
        setExportError(err instanceof Error ? err.message : "Export failed.")
      })
    return () => {
      cancelled = true
    }
  }, [image, quality, maxWidth, format])

  const resultBlob = image ? blob : null
  const previewUrl = useObjectUrl(resultBlob)
  const before = file?.size ?? 0
  const after = resultBlob?.size ?? 0
  const summary = useMemo(() => {
    if (!image || !resultBlob) return ""
    const size = computeScaledSize(
      image.naturalWidth,
      image.naturalHeight,
      maxWidth
    )
    return [
      `${size.width}×${size.height}`,
      `${formatByteSize(before)} → ${formatByteSize(after)}`,
      mimeForFormat(format),
    ].join("\n")
  }, [image, resultBlob, maxWidth, before, after, format])

  function handleFile(next: File | null) {
    setFile(next)
    setInput(next?.name ?? "")
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      inputLabel={t("input")}
      downloadFilename={withExtension(
        file?.name ?? "image",
        extForFormat(format)
      )}
      error={loadError ?? exportError}
      result={summary}
      onExample={() => {
        void makeSampleImageFile().then(handleFile)
      }}
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <label className={TOOL_LABEL_CLASS}>
              Quality
              <Input
                type="number"
                min={0.01}
                max={1}
                step={0.05}
                value={quality}
                onChange={(event) => setQuality(Number(event.target.value))}
                className="h-8 w-24"
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Max width
              <Input
                type="number"
                min={1}
                max={4096}
                value={maxWidth}
                onChange={(event) =>
                  setMaxWidth(Math.round(Number(event.target.value)) || 0)
                }
                className="h-8 w-28"
              />
            </label>
            <div className={TOOL_LABEL_CLASS}>
              JPEG / WebP / PNG
              <ToolboxSelect
                value={format}
                onChange={(value) => setFormat(value as ExportFormat)}
                options={EXPORT_FORMATS.map((item) => ({
                  value: item,
                  label: item.toUpperCase(),
                }))}
                className="w-full"
              />
            </div>
          </div>
        </div>
      }
      inputSlot={
        <Input
          key={input || "empty"}
          type="file"
          accept="image/*"
          onChange={(event) => handleFile(event.target.files?.[0] ?? null)}
        />
      }
      resultSlot={
        <ImageResultPreview
          url={previewUrl}
          meta={
            resultBlob
              ? `${formatByteSize(before)} → ${formatByteSize(after)}`
              : undefined
          }
          downloadLabel={t("download")}
          onDownload={() => {
            if (!resultBlob) return
            triggerBlobDownload(
              resultBlob,
              withExtension(file?.name ?? "image", extForFormat(format))
            )
          }}
        />
      }
    />
  )
}
