export type Rect = {
  x: number
  y: number
  w: number
  h: number
}

export type AspectPreset = "free" | "original" | "1:1" | "4:3" | "16:9" | "9:16"

export function aspectRatioFor(
  preset: AspectPreset,
  sourceW: number,
  sourceH: number
): number | null {
  if (preset === "free") return null
  if (preset === "original") {
    return sourceH === 0 ? null : sourceW / sourceH
  }
  if (preset === "1:1") return 1
  if (preset === "4:3") return 4 / 3
  if (preset === "16:9") return 16 / 9
  return 9 / 16
}

export function normalizeRect(rect: Rect): Rect {
  const x = rect.w < 0 ? rect.x + rect.w : rect.x
  const y = rect.h < 0 ? rect.y + rect.h : rect.y
  return { x, y, w: Math.abs(rect.w), h: Math.abs(rect.h) }
}

export function clampRect(rect: Rect, boundW: number, boundH: number): Rect {
  const n = normalizeRect(rect)
  const w = Math.min(Math.max(1, n.w), Math.max(1, boundW))
  const h = Math.min(Math.max(1, n.h), Math.max(1, boundH))
  const x = Math.min(Math.max(0, n.x), Math.max(0, boundW - w))
  const y = Math.min(Math.max(0, n.y), Math.max(0, boundH - h))
  return { x, y, w, h }
}

export function fitRectToAspect(
  rect: Rect,
  aspect: number,
  boundW: number,
  boundH: number
): Rect {
  const safeAspect = aspect > 0 ? aspect : 1
  let w = Math.max(1, rect.w)
  let h = w / safeAspect
  if (h > boundH) {
    h = boundH
    w = h * safeAspect
  }
  if (w > boundW) {
    w = boundW
    h = w / safeAspect
  }
  w = Math.max(1, w)
  h = Math.max(1, h)
  let x = rect.x
  let y = rect.y
  if (x + w > boundW) x = boundW - w
  if (y + h > boundH) y = boundH - h
  return clampRect({ x, y, w, h }, boundW, boundH)
}

export function mapDisplayRectToSource(
  display: Rect,
  scale: number,
  sourceW: number,
  sourceH: number
): Rect {
  const safeScale = scale > 0 ? scale : 1
  return clampRect(
    {
      x: display.x / safeScale,
      y: display.y / safeScale,
      w: display.w / safeScale,
      h: display.h / safeScale,
    },
    sourceW,
    sourceH
  )
}

export function mapSourceRectToDisplay(source: Rect, scale: number): Rect {
  const safeScale = scale > 0 ? scale : 1
  return {
    x: source.x * safeScale,
    y: source.y * safeScale,
    w: source.w * safeScale,
    h: source.h * safeScale,
  }
}

export function roundRect(rect: Rect): Rect {
  const x = Math.round(rect.x)
  const y = Math.round(rect.y)
  const w = Math.max(1, Math.round(rect.w))
  const h = Math.max(1, Math.round(rect.h))
  return { x, y, w, h }
}

export function fullImageRect(width: number, height: number): Rect {
  return {
    x: 0,
    y: 0,
    w: Math.max(1, width),
    h: Math.max(1, height),
  }
}

export function computeResampleSize(
  source: Rect,
  outputWidth: number
): { width: number; height: number } {
  const src = roundRect(source)
  const width = outputWidth > 0 ? Math.round(outputWidth) : Math.round(src.w)
  const height = Math.max(1, Math.round((src.h * width) / src.w))
  return { width: Math.max(1, width), height }
}
