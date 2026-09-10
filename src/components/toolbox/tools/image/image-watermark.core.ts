export type TileOrigin = { x: number; y: number }

export function iterTileOrigins(
  areaW: number,
  areaH: number,
  stampW: number,
  stampH: number,
  spacing: number
): TileOrigin[] {
  const stepX = stampW + spacing
  const stepY = stampH + spacing
  if (areaW <= 0 || areaH <= 0) return []
  if (stepX <= 0 || stepY <= 0) return [{ x: 0, y: 0 }]
  const origins: TileOrigin[] = []
  for (let y = 0; y < areaH; y += stepY) {
    for (let x = 0; x < areaW; x += stepX) {
      origins.push({ x, y })
    }
  }
  return origins
}

export function coverDiagonal(canvasW: number, canvasH: number): number {
  return Math.hypot(canvasW, canvasH)
}

export function clampOpacity(value: number): number {
  if (!Number.isFinite(value)) return 0.4
  return Math.min(1, Math.max(0, value))
}

export function clampAngle(value: number): number {
  if (!Number.isFinite(value)) return 0
  return Math.min(180, Math.max(-180, value))
}
