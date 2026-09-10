export type StitchMode = "horizontal" | "vertical" | "grid"

export type Size = {
  width: number
  height: number
}

export type StitchSlot = {
  x: number
  y: number
  width: number
  height: number
}

export type StitchLayout = {
  width: number
  height: number
  slots: StitchSlot[]
}

export function defaultGridColumns(count: number): number {
  if (count <= 0) return 1
  return Math.max(1, Math.ceil(Math.sqrt(count)))
}

export function computeStitchLayout(
  images: readonly Size[],
  mode: StitchMode,
  options: { gap?: number; columns?: number } = {}
): StitchLayout {
  if (images.length === 0) {
    return { width: 0, height: 0, slots: [] }
  }
  const gap = Math.max(0, Math.round(options.gap ?? 0))

  if (mode === "horizontal") {
    const height = Math.max(...images.map((img) => img.height))
    let x = 0
    const slots = images.map((img) => {
      const y = Math.round((height - img.height) / 2)
      const slot = { x, y, width: img.width, height: img.height }
      x += img.width + gap
      return slot
    })
    return { width: x - gap, height, slots }
  }

  if (mode === "vertical") {
    const width = Math.max(...images.map((img) => img.width))
    let y = 0
    const slots = images.map((img) => {
      const x = Math.round((width - img.width) / 2)
      const slot = { x, y, width: img.width, height: img.height }
      y += img.height + gap
      return slot
    })
    return { width, height: y - gap, slots }
  }

  const columns = Math.max(
    1,
    Math.min(
      images.length,
      Math.round(options.columns ?? defaultGridColumns(images.length))
    )
  )
  const rows = Math.ceil(images.length / columns)
  const cellW = Math.max(...images.map((img) => img.width))
  const cellH = Math.max(...images.map((img) => img.height))
  const slots = images.map((img, index) => {
    const col = index % columns
    const row = Math.floor(index / columns)
    const x = col * (cellW + gap) + Math.round((cellW - img.width) / 2)
    const y = row * (cellH + gap) + Math.round((cellH - img.height) / 2)
    return { x, y, width: img.width, height: img.height }
  })
  return {
    width: columns * cellW + (columns - 1) * gap,
    height: rows * cellH + (rows - 1) * gap,
    slots,
  }
}
