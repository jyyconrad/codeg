"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import {
  ToolboxCheckbox,
  ToolboxSelect,
} from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import {
  clampAngle,
  clampOpacity,
  coverDiagonal,
  iterTileOrigins,
} from "./image-watermark.core"
import { ImageResultPreview } from "./image-preview"
import { useLoadedImage } from "./use-loaded-image"
import { useObjectUrl } from "./use-object-url"
import {
  canvasToBlob,
  clampQuality,
  createExportCanvas,
  EXPORT_FORMATS,
  extForFormat,
  formatByteSize,
  makeSampleImageFile,
  mimeForFormat,
  TOOL_LABEL_CLASS,
  triggerBlobDownload,
  withExtension,
  type ExportFormat,
} from "./image-io"

function drawWatermark(
  ctx: CanvasRenderingContext2D,
  options: {
    canvasW: number
    canvasH: number
    text: string
    stamp: HTMLImageElement | null
    stampScale: number
    opacity: number
    angle: number
    spacing: number
    tile: boolean
    fill: string
    fontSize: number
  }
) {
  ctx.save()
  ctx.globalAlpha = clampOpacity(options.opacity)
  ctx.translate(options.canvasW / 2, options.canvasH / 2)
  ctx.rotate((clampAngle(options.angle) * Math.PI) / 180)
  ctx.textBaseline = "top"
  ctx.fillStyle = options.fill
  ctx.font = `${options.fontSize}px sans-serif`
  ctx.shadowColor = "rgba(0,0,0,0.45)"
  ctx.shadowBlur = 4

  let stampW: number
  let stampH: number
  if (options.stamp) {
    stampW = Math.max(1, options.stamp.naturalWidth * options.stampScale)
    stampH = Math.max(1, options.stamp.naturalHeight * options.stampScale)
  } else {
    stampW = Math.max(1, ctx.measureText(options.text).width)
    stampH = Math.max(1, options.fontSize)
  }

  const drawStamp = (x: number, y: number) => {
    if (options.stamp) {
      ctx.drawImage(options.stamp, x, y, stampW, stampH)
      return
    }
    ctx.fillText(options.text, x, y)
  }

  if (!options.tile) {
    drawStamp(-stampW / 2, -stampH / 2)
    ctx.restore()
    return
  }

  const diag = coverDiagonal(options.canvasW, options.canvasH)
  const origins = iterTileOrigins(
    diag,
    diag,
    stampW,
    stampH,
    Math.max(0, options.spacing)
  )
  for (const origin of origins) {
    drawStamp(origin.x - diag / 2, origin.y - diag / 2)
  }
  ctx.restore()
}

export default function ImageWatermarkTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("Watermark")
  const [file, setFile] = useState<File | null>(null)
  const [stampFile, setStampFile] = useState<File | null>(null)
  const [opacity, setOpacity] = useState(0.4)
  const [angle, setAngle] = useState(-24)
  const [spacing, setSpacing] = useState(96)
  const [tile, setTile] = useState(true)
  const [fill, setFill] = useState("#ffffff")
  const [fontSize, setFontSize] = useState(32)
  const [stampScale, setStampScale] = useState(0.25)
  const [format, setFormat] = useState<ExportFormat>("png")
  const [blob, setBlob] = useState<Blob | null>(null)
  const [exportError, setExportError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) setFile(null)
  }, [])
  useToolPendingInput(onInput)

  const { image, error: loadError } = useLoadedImage(file)
  const { image: stamp, error: stampError } = useLoadedImage(stampFile)
  const resultBlob = image ? blob : null
  const outUrl = useObjectUrl(resultBlob)

  useEffect(() => {
    if (!image) return
    let cancelled = false
    const mime = mimeForFormat(format)
    void (async () => {
      try {
        const { canvas, ctx } = createExportCanvas(
          image.naturalWidth,
          image.naturalHeight,
          mime
        )
        ctx.drawImage(image, 0, 0)
        drawWatermark(ctx, {
          canvasW: image.naturalWidth,
          canvasH: image.naturalHeight,
          text: input,
          stamp,
          stampScale,
          opacity,
          angle,
          spacing,
          tile,
          fill,
          fontSize,
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
  }, [
    image,
    stamp,
    input,
    opacity,
    angle,
    spacing,
    tile,
    fill,
    fontSize,
    stampScale,
    format,
  ])

  const summary = useMemo(() => {
    if (!image || !resultBlob) return ""
    return [
      `${image.naturalWidth}×${image.naturalHeight}`,
      formatByteSize(resultBlob.size),
      tile ? `tile ${spacing}px` : "single",
    ].join("\n")
  }, [image, resultBlob, tile, spacing])

  function handleFile(next: File | null) {
    setFile(next)
    if (next) setInput((prev) => prev || next.name)
    if (!next) setInput("")
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={(value) => {
        setInput(value)
        if (!value) {
          setFile(null)
          setStampFile(null)
        }
      }}
      inputLabel={t("input")}
      downloadFilename={withExtension(
        file?.name ?? "watermark",
        extForFormat(format)
      )}
      error={loadError ?? stampError ?? exportError}
      result={summary}
      onExample={() => {
        setInput("Watermark")
        void makeSampleImageFile(480, 320, "#1e293b", "Photo").then(handleFile)
      }}
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <label className={TOOL_LABEL_CLASS}>
              Opacity
              <Input
                type="number"
                min={0}
                max={1}
                step={0.05}
                className="h-8 w-24"
                value={opacity}
                onChange={(event) => setOpacity(Number(event.target.value))}
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Angle
              <Input
                type="number"
                min={-180}
                max={180}
                className="h-8 w-24"
                value={angle}
                onChange={(event) => setAngle(Number(event.target.value))}
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Spacing
              <Input
                type="number"
                min={0}
                className="h-8 w-24"
                value={spacing}
                onChange={(event) =>
                  setSpacing(Math.max(0, Number(event.target.value) || 0))
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Scale
              <Input
                type="number"
                min={0.05}
                max={2}
                step={0.05}
                className="h-8 w-20"
                value={stampScale}
                onChange={(event) =>
                  setStampScale(
                    Math.max(0.05, Number(event.target.value) || 0.05)
                  )
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Font
              <Input
                type="number"
                min={8}
                max={256}
                className="h-8 w-20"
                value={fontSize}
                onChange={(event) =>
                  setFontSize(Math.max(8, Number(event.target.value) || 8))
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Fill
              <Input
                type="color"
                className="h-8 w-16 p-1"
                value={fill}
                onChange={(event) => setFill(event.target.value)}
              />
            </label>
            <ToolboxCheckbox label="Tile" checked={tile} onChange={setTile} />
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
        <div className="flex min-h-[12rem] flex-1 flex-col gap-2">
          <Input
            value={input}
            onChange={(event) => setInput(event.target.value)}
            placeholder="Watermark"
          />
          <Input
            key={file?.name ?? "base"}
            type="file"
            accept="image/*"
            onChange={(event) => handleFile(event.target.files?.[0] ?? null)}
          />
          <Input
            key={stampFile?.name ?? "stamp"}
            type="file"
            accept="image/*"
            onChange={(event) => setStampFile(event.target.files?.[0] ?? null)}
          />
        </div>
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
              withExtension(file?.name ?? "watermark", extForFormat(format))
            )
          }}
        />
      }
    />
  )
}
