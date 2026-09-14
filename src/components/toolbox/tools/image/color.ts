export type Rgba = {
  r: number
  g: number
  b: number
  a: number
}

export type Hsla = {
  h: number
  s: number
  l: number
  a: number
}

export type ColorParse = {
  rgba: Rgba
  source: "hex" | "rgb" | "hsl"
}

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n))
}

function channelByte(n: number): number {
  return Math.round(clamp(n, 0, 255))
}

function hex2(n: number): string {
  return channelByte(n).toString(16).padStart(2, "0").toUpperCase()
}

export function parseHex(input: string): Rgba | null {
  const raw = input.trim().replace(/^#/, "")
  if (![3, 4, 6, 8].includes(raw.length)) return null
  if (!/^[0-9a-fA-F]+$/.test(raw)) return null
  const expand = raw.length < 6
  const parts = expand
    ? [...raw].map((ch) => ch + ch)
    : [raw.slice(0, 2), raw.slice(2, 4), raw.slice(4, 6), raw.slice(6, 8)]
  const r = Number.parseInt(parts[0], 16)
  const g = Number.parseInt(parts[1], 16)
  const b = Number.parseInt(parts[2], 16)
  const a =
    parts[3] != null && parts[3] !== ""
      ? Number.parseInt(parts[3], 16) / 255
      : 1
  return { r, g, b, a }
}

function parseAlphaToken(raw: string | undefined): number | null {
  if (raw == null || raw.trim() === "") return 1
  const token = raw.trim()
  const pct = token.endsWith("%")
  const n = Number(pct ? token.slice(0, -1) : token)
  if (!Number.isFinite(n)) return null
  if (pct) {
    if (n < 0 || n > 100) return null
    return n / 100
  }
  if (n < 0 || n > 1) return null
  return n
}

function parseRgbChannels(
  rRaw: string,
  gRaw: string,
  bRaw: string
): { r: number; g: number; b: number } | null {
  const r = Number(rRaw)
  const g = Number(gRaw)
  const b = Number(bRaw)
  if (![r, g, b].every(Number.isFinite)) return null
  if (r < 0 || r > 255 || g < 0 || g > 255 || b < 0 || b > 255) return null
  return { r: channelByte(r), g: channelByte(g), b: channelByte(b) }
}

export function parseRgb(input: string): Rgba | null {
  const trimmed = input.trim()
  const fn =
    /^(?:rgba?)\(\s*([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+)(?:\s*[,/]\s*([+-]?\d*\.?\d+%?))?\s*\)$/i.exec(
      trimmed
    )
  const plain =
    fn ??
    /^([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+)(?:\s*[,/]\s*([+-]?\d*\.?\d+%?))?$/.exec(
      trimmed
    )
  if (!plain) return null
  const rgb = parseRgbChannels(plain[1], plain[2], plain[3])
  if (!rgb) return null
  const a = parseAlphaToken(plain[4])
  if (a == null) return null
  return { ...rgb, a }
}

function parseHslChannels(
  hRaw: string,
  sRaw: string,
  lRaw: string
): { h: number; s: number; l: number } | null {
  const h = Number(hRaw)
  const s = Number(sRaw.replace(/%$/, ""))
  const l = Number(lRaw.replace(/%$/, ""))
  if (![h, s, l].every(Number.isFinite)) return null
  if (h < 0 || h > 360 || s < 0 || s > 100 || l < 0 || l > 100) return null
  return { h, s, l }
}

export function parseHsl(input: string): Hsla | null {
  const trimmed = input.trim()
  const fn =
    /^(?:hsla?)\(\s*([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+%?)\s*[, ]\s*([+-]?\d*\.?\d+%?)(?:\s*[,/]\s*([+-]?\d*\.?\d+%?))?\s*\)$/i.exec(
      trimmed
    )
  const plain =
    fn ??
    /^([+-]?\d*\.?\d+)\s*[, ]\s*([+-]?\d*\.?\d+%?)\s*[, ]\s*([+-]?\d*\.?\d+%?)(?:\s*[,/]\s*([+-]?\d*\.?\d+%?))?$/.exec(
      trimmed
    )
  if (!plain) return null
  const hsl = parseHslChannels(plain[1], plain[2], plain[3])
  if (!hsl) return null
  const a = parseAlphaToken(plain[4])
  if (a == null) return null
  return { ...hsl, a }
}

export function rgbToHsl(rgb: Rgba): Hsla {
  const r = clamp(rgb.r, 0, 255) / 255
  const g = clamp(rgb.g, 0, 255) / 255
  const b = clamp(rgb.b, 0, 255) / 255
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  const d = max - min
  let h = 0
  let s = 0
  if (d !== 0) {
    s = d / (1 - Math.abs(2 * l - 1))
    switch (max) {
      case r:
        h = ((g - b) / d) % 6
        break
      case g:
        h = (b - r) / d + 2
        break
      default:
        h = (r - g) / d + 4
        break
    }
    h *= 60
    if (h < 0) h += 360
  }
  return {
    h,
    s: s * 100,
    l: l * 100,
    a: clamp(rgb.a, 0, 1),
  }
}

export function hslToRgb(hsl: Hsla): Rgba {
  const h = ((hsl.h % 360) + 360) % 360
  const s = clamp(hsl.s, 0, 100) / 100
  const l = clamp(hsl.l, 0, 100) / 100
  const c = (1 - Math.abs(2 * l - 1)) * s
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1))
  const m = l - c / 2
  let r = 0
  let g = 0
  let b = 0
  if (h < 60) {
    r = c
    g = x
  } else if (h < 120) {
    r = x
    g = c
  } else if (h < 180) {
    g = c
    b = x
  } else if (h < 240) {
    g = x
    b = c
  } else if (h < 300) {
    r = x
    b = c
  } else {
    r = c
    b = x
  }
  return {
    r: channelByte((r + m) * 255),
    g: channelByte((g + m) * 255),
    b: channelByte((b + m) * 255),
    a: clamp(hsl.a, 0, 1),
  }
}

export function formatHex(rgb: Rgba): string {
  const base = `#${hex2(rgb.r)}${hex2(rgb.g)}${hex2(rgb.b)}`
  if (rgb.a >= 1 - 0.5 / 255) return base
  return `${base}${hex2(rgb.a * 255)}`
}

function formatAlpha(a: number): string {
  const rounded = Math.round(clamp(a, 0, 1) * 1000) / 1000
  return String(rounded)
}

export function formatRgb(rgb: Rgba): string {
  const r = channelByte(rgb.r)
  const g = channelByte(rgb.g)
  const b = channelByte(rgb.b)
  if (rgb.a >= 1 - 0.5 / 255) return `rgb(${r}, ${g}, ${b})`
  return `rgba(${r}, ${g}, ${b}, ${formatAlpha(rgb.a)})`
}

export function formatHsl(hsl: Hsla): string {
  const h = Math.round(hsl.h * 10) / 10
  const s = Math.round(hsl.s * 10) / 10
  const l = Math.round(hsl.l * 10) / 10
  if (hsl.a >= 1 - 0.5 / 255) return `hsl(${h}, ${s}%, ${l}%)`
  return `hsla(${h}, ${s}%, ${l}%, ${formatAlpha(hsl.a)})`
}

export function toCssColor(rgb: Rgba): string {
  return formatRgb(rgb)
}

export function parseColor(input: string): ColorParse | null {
  const trimmed = input.trim()
  if (!trimmed) return null
  const hex = parseHex(trimmed)
  if (hex) return { rgba: hex, source: "hex" }
  if (/^hsla?\(/i.test(trimmed)) {
    const hsl = parseHsl(trimmed)
    if (hsl) return { rgba: hslToRgb(hsl), source: "hsl" }
  }
  if (/^rgba?\(/i.test(trimmed)) {
    const rgb = parseRgb(trimmed)
    if (rgb) return { rgba: rgb, source: "rgb" }
  }
  const hsl = parseHsl(trimmed)
  if (hsl) return { rgba: hslToRgb(hsl), source: "hsl" }
  const rgb = parseRgb(trimmed)
  if (rgb) return { rgba: rgb, source: "rgb" }
  return null
}

export function colorError(input: string): string | null {
  if (!input.trim()) return null
  return parseColor(input) ? null : "Invalid HEX / RGB / HSL."
}
