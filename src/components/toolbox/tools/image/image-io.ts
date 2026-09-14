export const MAX_IMAGE_BYTES = 20 * 1024 * 1024
export const MAX_IMAGE_EDGE = 4096
export const IMAGE_LIMIT_MESSAGE = "Image exceeds 4096px or 20MB limit."
export const IMAGE_DECODE_MESSAGE = "Could not decode image."

export const EXPORT_FORMATS = ["jpeg", "png", "webp"] as const
export type ExportFormat = (typeof EXPORT_FORMATS)[number]

export const TOOL_LABEL_CLASS =
  "flex flex-col gap-1 text-xs font-medium text-muted-foreground"

export function guardImageFile(file: { size: number }): string | null {
  if (file.size > MAX_IMAGE_BYTES) return IMAGE_LIMIT_MESSAGE
  return null
}

export function guardImageSize(width: number, height: number): string | null {
  if (
    width < 1 ||
    height < 1 ||
    width > MAX_IMAGE_EDGE ||
    height > MAX_IMAGE_EDGE
  ) {
    return IMAGE_LIMIT_MESSAGE
  }
  return null
}

export function formatByteSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B"
  if (bytes < 1024) return `${Math.round(bytes)} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`
}

export function mimeForFormat(format: ExportFormat): string {
  if (format === "jpeg") return "image/jpeg"
  if (format === "webp") return "image/webp"
  return "image/png"
}

export function extForFormat(format: ExportFormat): string {
  if (format === "jpeg") return "jpg"
  if (format === "webp") return "webp"
  return "png"
}

export function withExtension(name: string, ext: string): string {
  const trimmed = name.trim()
  const stem = trimmed.replace(/\.[^.]+$/, "") || "image"
  return `${stem}.${ext}`
}

export function loadImageFromUrl(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => {
      if (img.naturalWidth < 1 || img.naturalHeight < 1) {
        reject(new Error(IMAGE_DECODE_MESSAGE))
        return
      }
      resolve(img)
    }
    img.onerror = () => reject(new Error(IMAGE_DECODE_MESSAGE))
    img.src = url
  })
}

export function canvasToBlob(
  canvas: HTMLCanvasElement,
  type: string,
  quality?: number
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (!blob) {
          reject(new Error("Export failed."))
          return
        }
        resolve(blob)
      },
      type,
      quality
    )
  })
}

export function createExportCanvas(
  width: number,
  height: number,
  mime: string
): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D } {
  const limited = guardImageSize(width, height)
  if (limited) throw new Error(limited)
  const canvas = document.createElement("canvas")
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext("2d")
  if (!ctx) throw new Error("Export failed.")
  if (mime === "image/jpeg") {
    ctx.fillStyle = "#ffffff"
    ctx.fillRect(0, 0, width, height)
  }
  return { canvas, ctx }
}

export function triggerBlobDownload(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob)
  const link = document.createElement("a")
  link.href = url
  link.download = filename
  document.body.append(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}

export async function blobFromCanvasImage(
  image: CanvasImageSource,
  width: number,
  height: number,
  format: ExportFormat,
  quality: number
): Promise<Blob> {
  const mime = mimeForFormat(format)
  const { canvas, ctx } = createExportCanvas(width, height, mime)
  ctx.imageSmoothingEnabled = true
  ctx.imageSmoothingQuality = "high"
  ctx.drawImage(image, 0, 0, width, height)
  return canvasToBlob(
    canvas,
    mime,
    format === "png" ? undefined : clampQuality(quality)
  )
}

export function clampQuality(quality: number): number {
  if (!Number.isFinite(quality)) return 0.8
  return Math.min(1, Math.max(0.01, quality))
}

export function computeScaledSize(
  width: number,
  height: number,
  maxWidth: number
): { width: number; height: number } {
  if (maxWidth <= 0 || width <= maxWidth) {
    return { width, height }
  }
  return {
    width: maxWidth,
    height: Math.max(1, Math.round((height * maxWidth) / width)),
  }
}

export async function makeSampleImageFile(
  width = 320,
  height = 200,
  fill = "#2563eb",
  label = "Sample",
  name = "sample.png"
): Promise<File> {
  const { canvas, ctx } = createExportCanvas(width, height, "image/png")
  ctx.fillStyle = fill
  ctx.fillRect(0, 0, width, height)
  ctx.fillStyle = "#ffffff"
  ctx.font = `${Math.max(16, Math.round(height / 6))}px sans-serif`
  ctx.textBaseline = "middle"
  ctx.fillText(label, 16, height / 2)
  const blob = await canvasToBlob(canvas, "image/png")
  return new File([blob], name, { type: "image/png" })
}
