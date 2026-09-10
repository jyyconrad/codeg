"use client"

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react"
import { useTranslations } from "next-intl"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import {
  aspectRatioFor,
  clampRect,
  computeResampleSize,
  fitRectToAspect,
  fullImageRect,
  mapDisplayRectToSource,
  mapSourceRectToDisplay,
  roundRect,
  type AspectPreset,
  type Rect,
} from "./image-crop.core"
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

const ASPECTS: AspectPreset[] = [
  "free",
  "original",
  "1:1",
  "4:3",
  "16:9",
  "9:16",
]

function CropStage({
  image,
  previewUrl,
  rect,
  aspect,
  onRectChange,
}: {
  image: HTMLImageElement
  previewUrl: string
  rect: Rect
  aspect: number | null
  onRectChange: (rect: Rect) => void
}) {
  const stageRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<{
    originX: number
    originY: number
  } | null>(null)
  const [display, setDisplay] = useState({ w: 0, h: 0 })
  const scale = display.w > 0 ? display.w / image.naturalWidth : 1
  const overlay = mapSourceRectToDisplay(rect, scale)

  function clientToDisplay(event: ReactPointerEvent<HTMLDivElement>) {
    const box = stageRef.current?.getBoundingClientRect()
    if (!box) return { x: 0, y: 0 }
    return {
      x: event.clientX - box.left,
      y: event.clientY - box.top,
    }
  }

  function applyDisplayRect(displayRect: Rect) {
    let next = mapDisplayRectToSource(
      displayRect,
      scale,
      image.naturalWidth,
      image.naturalHeight
    )
    if (aspect) {
      next = fitRectToAspect(
        next,
        aspect,
        image.naturalWidth,
        image.naturalHeight
      )
    }
    onRectChange(next)
  }

  return (
    <div
      ref={stageRef}
      className="relative inline-block max-w-full cursor-crosshair touch-none"
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId)
        const point = clientToDisplay(event)
        dragRef.current = { originX: point.x, originY: point.y }
        applyDisplayRect({ x: point.x, y: point.y, w: 1, h: 1 })
      }}
      onPointerMove={(event) => {
        const drag = dragRef.current
        if (!drag) return
        const point = clientToDisplay(event)
        applyDisplayRect({
          x: drag.originX,
          y: drag.originY,
          w: point.x - drag.originX,
          h: point.y - drag.originY,
        })
      }}
      onPointerUp={() => {
        dragRef.current = null
      }}
    >
      {/* Local blob preview; next/image cannot take object URLs. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src={previewUrl}
        alt=""
        draggable={false}
        className="max-h-[24rem] max-w-full select-none"
        onLoad={(event) => {
          setDisplay({
            w: event.currentTarget.clientWidth,
            h: event.currentTarget.clientHeight,
          })
        }}
      />
      {display.w > 0 ? (
        <div
          className="pointer-events-none absolute border-2 border-primary bg-primary/15"
          style={{
            left: overlay.x,
            top: overlay.y,
            width: overlay.w,
            height: overlay.h,
          }}
        />
      ) : null}
    </div>
  )
}

export default function ImageCropTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [file, setFile] = useState<File | null>(null)
  const [rectOverride, setRectOverride] = useState<Rect | null>(null)
  const [preset, setPreset] = useState<AspectPreset>("free")
  const [outputWidth, setOutputWidth] = useState(0)
  const [format, setFormat] = useState<ExportFormat>("png")
  const quality = 0.92
  const [blob, setBlob] = useState<Blob | null>(null)
  const [exportError, setExportError] = useState<string | null>(null)
  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) {
      setFile(null)
      setRectOverride(null)
    }
  }, [])
  useToolPendingInput(onInput)

  const { image, previewUrl, error: loadError } = useLoadedImage(file)
  const aspect = image
    ? aspectRatioFor(preset, image.naturalWidth, image.naturalHeight)
    : null
  const defaultRect = useMemo(() => {
    if (!image) return fullImageRect(1, 1)
    const full = fullImageRect(image.naturalWidth, image.naturalHeight)
    return aspect
      ? fitRectToAspect(full, aspect, image.naturalWidth, image.naturalHeight)
      : full
  }, [image, aspect])
  const rect = rectOverride ?? defaultRect
  const resultBlob = image ? blob : null
  const outUrl = useObjectUrl(resultBlob)

  useEffect(() => {
    if (!image) return
    let cancelled = false
    const source = roundRect(
      clampRect(rect, image.naturalWidth, image.naturalHeight)
    )
    const size = computeResampleSize(source, outputWidth)
    const mime = mimeForFormat(format)
    void (async () => {
      try {
        const { canvas, ctx } = createExportCanvas(
          size.width,
          size.height,
          mime
        )
        ctx.imageSmoothingEnabled = true
        ctx.imageSmoothingQuality = "high"
        ctx.drawImage(
          image,
          source.x,
          source.y,
          source.w,
          source.h,
          0,
          0,
          size.width,
          size.height
        )
        const next = await canvasToBlob(
          canvas,
          mime,
          format === "png" ? undefined : clampQuality(quality)
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
  }, [image, rect, outputWidth, format, quality])

  const sourcePx = roundRect(
    image ? clampRect(rect, image.naturalWidth, image.naturalHeight) : rect
  )
  const outSize = computeResampleSize(sourcePx, outputWidth)
  const summary = useMemo(() => {
    if (!image || !resultBlob) return ""
    return [
      `crop ${Math.round(sourcePx.x)},${Math.round(sourcePx.y)} ${Math.round(sourcePx.w)}×${Math.round(sourcePx.h)}`,
      `export ${outSize.width}×${outSize.height}`,
      formatByteSize(resultBlob.size),
    ].join("\n")
  }, [image, resultBlob, sourcePx, outSize])

  function handleFile(next: File | null) {
    setFile(next)
    setInput(next?.name ?? "")
    setRectOverride(null)
  }

  function updateRect(next: Rect) {
    if (!image) {
      setRectOverride(next)
      return
    }
    const clamped = clampRect(next, image.naturalWidth, image.naturalHeight)
    setRectOverride(
      aspect
        ? fitRectToAspect(
            clamped,
            aspect,
            image.naturalWidth,
            image.naturalHeight
          )
        : clamped
    )
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      inputLabel={t("input")}
      downloadFilename={withExtension(
        file?.name ?? "crop",
        extForFormat(format)
      )}
      error={loadError ?? exportError}
      result={summary}
      onExample={() => {
        void makeSampleImageFile(480, 320, "#0f766e", "Crop").then(handleFile)
      }}
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <div className={TOOL_LABEL_CLASS}>
              Aspect
              <ToolboxSelect
                value={preset}
                onChange={(value) => {
                  setPreset(value as AspectPreset)
                  setRectOverride(null)
                }}
                options={ASPECTS.map((item) => ({
                  value: item,
                  label: item,
                }))}
                className="w-full"
              />
            </div>
            <label className={TOOL_LABEL_CLASS}>
              x
              <Input
                type="number"
                className="h-8 w-20"
                value={Math.round(sourcePx.x)}
                onChange={(event) =>
                  updateRect({ ...sourcePx, x: Number(event.target.value) })
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              y
              <Input
                type="number"
                className="h-8 w-20"
                value={Math.round(sourcePx.y)}
                onChange={(event) =>
                  updateRect({ ...sourcePx, y: Number(event.target.value) })
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              w
              <Input
                type="number"
                className="h-8 w-20"
                value={Math.round(sourcePx.w)}
                onChange={(event) =>
                  updateRect({ ...sourcePx, w: Number(event.target.value) })
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              h
              <Input
                type="number"
                className="h-8 w-20"
                value={Math.round(sourcePx.h)}
                onChange={(event) =>
                  updateRect({ ...sourcePx, h: Number(event.target.value) })
                }
              />
            </label>
            <label className={TOOL_LABEL_CLASS}>
              Export width
              <Input
                type="number"
                min={0}
                max={4096}
                className="h-8 w-28"
                value={outputWidth}
                onChange={(event) =>
                  setOutputWidth(Math.max(0, Number(event.target.value) || 0))
                }
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
        <div className="flex min-h-[12rem] flex-1 flex-col gap-2">
          <Input
            key={input || "empty"}
            type="file"
            accept="image/*"
            onChange={(event) => handleFile(event.target.files?.[0] ?? null)}
          />
          {image && previewUrl ? (
            <CropStage
              image={image}
              previewUrl={previewUrl}
              rect={sourcePx}
              aspect={aspect}
              onRectChange={updateRect}
            />
          ) : null}
        </div>
      }
      resultSlot={
        <ImageResultPreview
          url={outUrl}
          meta={
            resultBlob
              ? `${outSize.width}×${outSize.height} · ${formatByteSize(resultBlob.size)}`
              : undefined
          }
          downloadLabel={t("download")}
          onDownload={() => {
            if (!resultBlob) return
            triggerBlobDownload(
              resultBlob,
              withExtension(file?.name ?? "crop", extForFormat(format))
            )
          }}
        />
      }
    />
  )
}
